use std::env;
use std::error::Error;
use std::str::FromStr;

use chrono::{Duration, SecondsFormat, Utc};
use lambda::error::HandlerError;
use lambda::lambda;
use rusoto_cloudwatch::{CloudWatch, CloudWatchClient, Dimension, GetMetricStatisticsInput};
use rusoto_core::Region;
use rusoto_ssm::{GetParameterRequest, Ssm, SsmClient};
use serde_derive::{Deserialize, Serialize};
use slack_hook::{PayloadBuilder, Slack};

#[derive(Deserialize, Clone)]
struct CustomEvent {}

#[derive(Serialize, Clone)]
struct CustomOutput {}

struct CloudWatchFacade {
    client: CloudWatchClient,
}

impl CloudWatchFacade {
    fn new(client: CloudWatchClient) -> Self {
        CloudWatchFacade { client }
    }

    fn get_total_cost(&self, c: &lambda::Context) -> Result<f64, HandlerError> {
        let duration = Duration::days(1);
        let end_time = Utc::now();
        let start_time = end_time - duration;
        let metric = self.client.get_metric_statistics(GetMetricStatisticsInput {
            dimensions: Some(vec![Dimension {
                name: "Currency".to_string(),
                value: "USD".to_string(),
            }]),
            metric_name: "EstimatedCharges".to_string(),
            namespace: "AWS/Billing".to_string(),
            statistics: Some(vec!["Maximum".to_string()]),
            start_time: start_time.to_rfc3339_opts(SecondsFormat::Secs, true),
            end_time: end_time.to_rfc3339_opts(SecondsFormat::Secs, true),
            period: duration.num_seconds(),
            extended_statistics: None,
            unit: None,
        });

        match metric.sync() {
            Err(err) => Err(c.new_error(&err.to_string())),
            Ok(metric) => Ok(metric
                .datapoints
                .map(|dp| {
                    if dp.is_empty() {
                        return 0.0;
                    }
                    return dp[0].maximum.unwrap_or(0.0);
                })
                .unwrap_or(0.0)),
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    openssl_probe::init_ssl_cert_env_vars();
    simple_logger::init_with_level(log::Level::Info)?;
    lambda!(my_handler);

    Ok(())
}

fn my_handler(_e: CustomEvent, c: lambda::Context) -> Result<CustomOutput, HandlerError> {
    let client = CloudWatchFacade::new(CloudWatchClient::new(Region::UsEast1));
    let total = client.get_total_cost(&c)?;
    send_to_slack(&c, total)?;

    Ok(CustomOutput {})
}

fn send_to_slack(c: &lambda::Context, total: f64) -> Result<(), HandlerError> {
    let ssm_region = match env::var("AWS_SSM_REGION") {
        Ok(region) => Region::from_str(region.as_str()).unwrap(),
        Err(err) => return Err(c.new_error(err.description())),
    };
    let ssm = SsmClient::new(ssm_region);
    let ssm_result = ssm.get_parameter(GetParameterRequest {
        name: "/billing-notification/slack-webhook-url".to_string(),
        with_decryption: Some(true),
    });
    let webhook_url = match ssm_result.sync() {
        Err(err) => return Err(c.new_error(err.description())),
        Ok(res) => res.parameter.map(|p| p.value.unwrap()).unwrap(),
    };

    let slack = Slack::new(webhook_url.as_str()).unwrap();
    let payload = PayloadBuilder::new()
        .username("AWS Billing Notification")
        .icon_emoji(":money_with_wings:")
        .text(format!("今月の請求額は ${} です", total))
        .build()
        .unwrap();
    let res = slack.send(&payload);

    match res {
        Ok(_) => Ok(()),
        Err(err) => Err(c.new_error(err.description())),
    }
}
