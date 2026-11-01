//! Reddit threads through the readers in `reddit.reader`, then a search
//! index's copy. The logic lives in `providers::reddit`, which the reddit
//! search mode shares.

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

    fn billed_calls(&self) -> Vec<String> {
        self.take_billed_calls()
    }
}
