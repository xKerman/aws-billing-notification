# aws-billing-notification

## development

### preparation

Tools:
* [Slack](https://slack.com/) workspace is needed for notification
* [Rust](https://www.rust-lang.org/) is needed for compiling code
* [Docker](https://www.docker.com/) is needed for cross compiling
* [AWS CLI](https://aws.amazon.com/cli/) is needed for deployment

AWS Environment:
* [Amazon S3](https://aws.amazon.com/s3/) bucket is needed for putting AWS Lambda code
* Put [Slack Incoming Webhook](https://api.slack.com/incoming-webhooks) URL into [AWS System Manager Parameter Store](https://docs.aws.amazon.com/systems-manager/latest/userguide/systems-manager-paramstore.html)
    * Name: `/billing-notification/slack-webhook-url`
    * Type: `SafeString`

### build

```
$ make build # to create zip file for AWS Lambda function
```

### deploy

```
$ make deploy AWS_S3_BUCKET=<your s3 bucket name>
```
