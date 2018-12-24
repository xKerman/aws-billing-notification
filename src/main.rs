use std::env;
use std::error::Error;
use std::str::FromStr;

use chrono::{Duration, SecondsFormat, Utc};
use lambda::error::HandlerError;
use lambda::lambda;
use rusoto_cloudwatch::{
    CloudWatch, CloudWatchClient, Dimension, DimensionFilter, GetMetricStatisticsInput,
    ListMetricsInput,
};
use rusoto_core::Region;
use rusoto_ssm::{GetParameterRequest, Ssm, SsmClient};
use serde_derive::{Deserialize, Serialize};
use slack_hook::{AttachmentBuilder, Field, PayloadBuilder, Slack};

#[derive(Deserialize, Clone)]
struct CustomEvent {}

#[derive(Serialize, Clone)]
struct CustomOutput {}

struct CloudWatchFacade<'a> {
    context: &'a lambda::Context,
    client: CloudWatchClient,
}

struct Billing {
    total: f64,
    services: Vec<ServiceBilling>,
}

struct ServiceBilling {
    name: String,
    cost: f64,
}

impl<'a> CloudWatchFacade<'a> {
    fn new(context: &'a lambda::Context, client: CloudWatchClient) -> Self {
        CloudWatchFacade { context, client }
    }

    fn get_total_cost(&self) -> Result<f64, HandlerError> {
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
            Err(err) => Err(self.context.new_error(&err.to_string())),
            Ok(metric) => Ok(metric
                .datapoints
                .map(|dp| {
                    if dp.is_empty() {
                        return 0.0;
                    }
                    dp[0].maximum.unwrap_or(0.0)
                })
                .unwrap_or(0.0)),
        }
    }

    fn get_services_in_billing_namespace(&self) -> Result<Vec<String>, HandlerError> {
        let output = self.client.list_metrics(ListMetricsInput {
            namespace: Some("AWS/Billing".to_string()),
            dimensions: Some(vec![DimensionFilter {
                name: "ServiceName".to_string(),
                value: None,
            }]),
            metric_name: None,
            next_token: None,
        });

        match output.sync() {
            Err(err) => Err(self.context.new_error(err.description())),
            Ok(output) => {
                let metrics = output.metrics.unwrap_or_default();
                Ok(metrics
                    .into_iter()
                    .flat_map(|m| m.dimensions.unwrap_or_default())
                    .filter_map(|d| {
                        if d.name == "ServiceName" {
                            return Some(d.value);
                        }
                        None
                    })
                    .collect())
            }
        }
    }

    fn get_cost(&self, service: &str) -> Result<ServiceBilling, HandlerError> {
        let duration = Duration::days(1);
        let end_time = Utc::now();
        let start_time = end_time - duration;
        let metric = self.client.get_metric_statistics(GetMetricStatisticsInput {
            dimensions: Some(vec![
                Dimension {
                    name: "Currency".to_string(),
                    value: "USD".to_string(),
                },
                Dimension {
                    name: "ServiceName".to_string(),
                    value: service.to_string(),
                },
            ]),
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
            Err(err) => Err(self.context.new_error(&err.to_string())),
            Ok(metric) => {
                let cost = metric
                    .datapoints
                    .map(|dp| {
                        if dp.is_empty() {
                            return 0.0;
                        }
                        dp[0].maximum.unwrap_or(0.0)
                    })
                    .unwrap_or(0.0);
                Ok(ServiceBilling {
                    name: service.to_string(),
                    cost,
                })
            }
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    openssl_probe::init_ssl_cert_env_vars();
    simple_logger::init_with_level(log::Level::Info)?;
    lambda!(my_handler);

    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn my_handler(_e: CustomEvent, c: lambda::Context) -> Result<CustomOutput, HandlerError> {
    let client = CloudWatchFacade::new(&c, CloudWatchClient::new(Region::UsEast1));
    let total = client.get_total_cost()?;
    let services = client.get_services_in_billing_namespace()?;
    let costs = services
        .iter()
        .map(|service| client.get_cost(&service))
        .collect::<Result<Vec<_>, _>>()?;
    let billing = Billing {
        total,
        services: costs,
    };
    send_to_slack(&c, billing)?;

    Ok(CustomOutput {})
}

fn send_to_slack(c: &lambda::Context, billing: Billing) -> Result<(), HandlerError> {
    let ssm_region = match env::var("AWS_REGION") {
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

    let attachments = AttachmentBuilder::new("each service")
        .fields(
            billing
                .services
                .into_iter()
                .map(|service| Field::new(service.name, format!("${}", service.cost), Some(true)))
                .collect(),
        )
        .build()
        .unwrap();
    let payload = PayloadBuilder::new()
        .username("AWS Billing Notification")
        .icon_emoji(":money_with_wings:")
        .text(format!("今月の請求額は ${} です", billing.total))
        .attachments(vec![attachments])
        .build()
        .unwrap();
    let slack = Slack::new(webhook_url.as_str()).unwrap();
    let res = slack.send(&payload);

    match res {
        Ok(_) => Ok(()),
        Err(err) => Err(c.new_error(err.description())),
    }
}
