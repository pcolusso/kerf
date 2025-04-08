use aws_sdk_cloudwatchlogs as cwl;

#[derive(Clone)]
pub struct Logs {
    client: cwl::Client,
    group: String,
    query: Option<String>,
    next_token: Option<String>,
    // kludge, we need to identify which stream we need.
    stream: Option<String>,
}

// This looks a lot like an iterator, or a cursor...
impl Logs {
    pub async fn new(group: String) -> Self {
        let config = aws_config::load_from_env().await;
        let client = cwl::Client::new(&config);
        let query = None;
        Self { client, group, query, next_token: None, stream: None }
    }

    async fn get_first_stream(&self) -> Option<String> {
        let req = self.client.describe_log_streams()
            .order_by(aws_sdk_cloudwatchlogs::types::OrderBy::LastEventTime)
            .descending(true)
            .log_group_name(&self.group)
            .limit(1);
        let res = req.send().await.unwrap();
        res.log_streams()[0].log_stream_name().map(|s| s.to_owned())
    }

    pub fn set_query(&mut self, query: String) {
        self.query = Some(query);
        self.next_token = None;
    }

    pub async fn find_context(&mut self, middle: i64) -> Vec<String> {
        let start_time = middle - 1000;
        let end_time = middle + 1000;
        let req = self.client.get_log_events()
            .log_group_name(&self.group)
            .log_stream_name(self.stream.clone().expect("Stream not set"))
            .start_time(start_time)
            .end_time(end_time)
            .limit(40);
        let res = req.send().await.unwrap();
        let mut evts = Vec::new();
        for event in res.events() {
            if let Some(y) = event.message() {
                evts.push(y.to_owned());
            }
        }
        evts
    }

    // idk, should we handle back & forward? For now, assume ALWAYS FORWARD
    pub async fn get_more_logs(&mut self) -> Vec<(i64, String)> {
        self.stream = self.get_first_stream().await;
        let req = self.client.filter_log_events()
            .log_group_name(&self.group)
            .set_log_stream_names(Some(vec![self.stream.clone().unwrap()]))
            .limit(40)
            .set_filter_pattern(self.query.clone())
            .set_next_token(self.next_token.clone());
        let res = req.send().await.unwrap();
        self.next_token = res.next_token().map(|s| s.into());
        // What's more correct, returning an empty vec, or an option and acually dealing with it...
        let mut evts = Vec::new();
        for event in res.events() {
            if let (Some(x), Some(y)) = (event.timestamp(), event.message()) {
                evts.push((x, y.to_owned()));
            }
        }
        evts
    }
}
