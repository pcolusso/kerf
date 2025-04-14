use aws_sdk_cloudwatchlogs as cwl;
use anyhow::Result;

pub struct Logs {
    client: cwl::Client,
    group: String,
    query: Option<String>,
    next_token: Option<String>,
    // kludge, we need to identify which stream we need.
    stream: String,
    snip: usize
}

// TODO: Fix this junk
async fn get_first_stream(client: &cwl::Client, group: &str) -> Result<String> {
    let req = client.describe_log_streams()
        .order_by(aws_sdk_cloudwatchlogs::types::OrderBy::LastEventTime)
        .descending(true)
        .log_group_name(group)
        .limit(1);
    let res = req.send().await?;
    res.log_streams()[0].log_stream_name().map(|s| s.to_owned()).ok_or(anyhow::anyhow!("Can't find a log stream."))
}

// This looks a lot like an iterator, or a cursor...
impl Logs {
    pub async fn new(group: String, stream: Option<String>, snip: Option<usize>) -> Result<Self> {
        let config = aws_config::load_from_env().await;
        let client = cwl::Client::new(&config);
        let query = None;
        let stream = match stream {
            None => get_first_stream(&client, &group).await.expect("Couldn't find stream"),
            Some(s) => s,
        };
        Ok(Self { client, group, query, next_token: None, stream, snip: snip.unwrap_or(0) })
    }

    pub fn set_query(&mut self, query: String) {
        self.query = Some(query);
        self.next_token = None;
    }

    pub async fn find_context(&mut self, middle: i64) -> Result<Vec<String>> {
        // TODO: Should this be configurable?
        let start_time = middle - 1000;
        let end_time = middle + 1000;
        let req = self.client.get_log_events()
            .log_group_name(&self.group)
            .log_stream_name(self.stream.clone())
            .start_time(start_time)
            .end_time(end_time)
            .limit(40);
        let res = req.send().await?;
        let mut evts = Vec::new();
        for event in res.events() {
            if let Some(y) = event.message() {
                evts.push(y.chars().skip(self.snip).collect());
            }
        }
        Ok(evts)
    }

    pub async fn get_more_logs(&mut self) -> Result<Vec<(i64, String)>> {
        let req = self.client.filter_log_events()
            .log_group_name(&self.group)
            .set_log_stream_names(Some(vec![self.stream.clone()]))
            .limit(1000) // Probably could obviate the need for pagination with a suitably large ret 😈
            .set_filter_pattern(self.query.clone())
            
            // Okay to take, considering we're about to put something else in, right?
            .set_next_token(self.next_token.take());
        let res = req.send().await?;
        self.next_token = res.next_token().map(|s| s.into());
        // What's more correct, returning an empty vec, or an option and acually dealing with it...
        let mut evts = Vec::new();
        for event in res.events() {
            if let (Some(x), Some(y)) = (event.timestamp(), event.message()) {
                evts.push((x, y.chars().skip(self.snip).collect()));
            }
        }
        Ok(evts)
    }
}
