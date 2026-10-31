//! Reddit threads from a search index's copy; reddit.com itself is never
//! requested. The logic lives in `providers::reddit`, which the reddit search
//! mode shares.

use super::Route;
use crate::errors::SearchError;
use crate::providers::reddit::{self, Reddit};
use crate::types::SearchResult;
use async_trait::async_trait;
use url::Url;

#[async_trait]
impl Route for Reddit {
    fn name(&self) -> &'static str {
        Reddit::NAME
    }

    fn matches(&self, url: &Url) -> bool {
        reddit::is_reddit_url(url)
    }

    async fn fetch(&self, url: &Url) -> Result<Vec<SearchResult>, SearchError> {
        self.fetch_thread(url).await
    }
}
