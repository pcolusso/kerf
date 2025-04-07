use aws_sdk_cloudwatchlogs as cwl;

#[derive(Clone)]
pub struct Logs {
    client: cwl::Client,
    group: String,
    query: Option<String>,
    next_token: Option<String>,
}

// This looks a lot like an iterator, or a cursor...
impl Logs {
    pub async fn new(group: String) -> Self {
        let config = aws_config::load_from_env().await;
        let client = cwl::Client::new(&config);
        let query = None;
        Self { client, group, query, next_token: None }
    }

    pub fn set_query(&mut self, query: String) {
        self.query = Some(query);
        self.next_token = None;
    }

    // idk, should we handle back & forward? For now, assume ALWAYS FORWARD
    pub async fn get_more_logs(&mut self) -> Vec<(i64, String)> {
        let req = self.client.filter_log_events()
            .log_group_name(&self.group)
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
