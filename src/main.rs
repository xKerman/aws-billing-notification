#[macro_use]
extern crate lambda_runtime as lambda;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate log;
extern crate simple_logger;
extern crate rusoto_core;
extern crate chrono;

use std::error::Error;

use chrono::{Duration, SecondsFormat, Utc};
use lambda::error::HandlerError;
use rusoto_core::Region;
use rusoto_cloudwatch::{CloudWatch, CloudWatchClient, Dimension, GetMetricStatisticsInput};

#[derive(Deserialize, Clone)]
struct CustomEvent {
    #[serde(rename = "firstName")]
    first_name: String,
}

#[derive(Serialize, Clone)]
struct CustomOutput {
    message: String,
    total: f64,
}

fn main() -> Result<(), Box<dyn Error>> {
    simple_logger::init_with_level(log::Level::Info)?;
    lambda!(my_handler);

    Ok(())
}


fn my_handler(e: CustomEvent, c: lambda::Context) -> Result<CustomOutput, HandlerError> {
    let duration = Duration::days(1);
    let end_time = Utc::now();
    let start_time = end_time - duration;
    let client = CloudWatchClient::new(Region::UsEast1);
    let metric = client.get_metric_statistics(GetMetricStatisticsInput {
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
    let total = match metric.sync() {
        Err(err) => return Err(c.new_error(&err.to_string())),
        Ok(metric) => metric.datapoints.map(|dp| {
            if dp.is_empty() {
                return 0.0;
            }
            return dp[0].maximum.unwrap_or(0.0);
        }).unwrap_or(0.0),
    };
    if e.first_name == "" {
        error!("Empty first name in request {}", c.aws_request_id);
        return Err(c.new_error("Empty first name"));
    }

    Ok(CustomOutput {
        message: format!("Hello, {}!", e.first_name),
        total: total,
    })
}
