use crate::parse::build_ingestion_batch;
use crate::spec::{BibleSource, BibleSourceFormat, BibleTranslationSpec};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tracing::warn;

#[async_trait]
pub trait BibleContentProvider: Send + Sync {
    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>>;
}

#[derive(Clone, Debug)]
pub struct HttpBibleContentProvider {
    client: reqwest::Client,
}

impl HttpBibleContentProvider {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent("PresenterBibleScraper/0.1")
            .build()?;
        Ok(Self { client })
    }
}

fn resolve_cache_path(url: &str) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PRESENTER_BIBLE_CACHE_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join(sanitise_url(url)));
        }
    }
    if let Some(base) = std::env::var_os("XDG_CACHE_HOME") {
        let mut path = PathBuf::from(base);
        path.push("presenter");
        path.push("bibles");
        path.push(sanitise_url(url));
        return Some(path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let mut path = PathBuf::from(home);
        path.push(".cache");
        path.push("presenter");
        path.push("bibles");
        path.push(sanitise_url(url));
        return Some(path);
    }
    None
}

fn sanitise_url(url: &str) -> String {
    url.chars()
        .map(|ch| match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '-' => ch,
            _ => '_',
        })
        .collect()
}

#[async_trait]
impl BibleContentProvider for HttpBibleContentProvider {
    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let cache_path = resolve_cache_path(url);
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 0..3 {
            match self.client.get(url).send().await {
                Ok(response) => match response.error_for_status() {
                    Ok(success) => match success.bytes().await {
                        Ok(bytes) => {
                            let bytes = bytes.to_vec();
                            if let Some(path) = cache_path.as_ref() {
                                if let Some(parent) = path.parent() {
                                    if let Err(err) = fs::create_dir_all(parent) {
                                        warn!(?err, "failed to create bible cache directory");
                                    }
                                }
                                if let Err(err) = fs::write(path, &bytes) {
                                    warn!(?err, path = %path.display(), "failed to write bible cache");
                                }
                            }
                            return Ok(bytes);
                        }
                        Err(err) => {
                            last_error = Some(err.into());
                        }
                    },
                    Err(err) => {
                        last_error = Some(err.into());
                    }
                },
                Err(err) => {
                    last_error = Some(err.into());
                }
            }

            if attempt < 2 {
                let backoff = Duration::from_secs(2_u64.pow(attempt as u32));
                tokio::time::sleep(backoff).await;
            }
        }

        if let Some(path) = cache_path.as_ref() {
            if path.exists() {
                warn!(
                    path = %path.display(),
                    error = ?last_error,
                    "using cached bible archive after download failure"
                );
                return fs::read(path).with_context(|| {
                    format!(
                        "failed to read cached bible archive from {}",
                        path.display()
                    )
                });
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("failed to download bible archive")))
    }
}

pub struct BibleScraper<P: BibleContentProvider> {
    provider: P,
}

impl<P: BibleContentProvider> BibleScraper<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }

    pub async fn scrape(
        &self,
        spec: &BibleTranslationSpec,
    ) -> Result<(
        presenter_core::bible::BibleIngestionBatch,
        crate::spec::BibleImportSummary,
    )> {
        match (&spec.source, &spec.format) {
            (
                BibleSource::Url { url },
                BibleSourceFormat::UsfmZip { .. }
                | BibleSourceFormat::MySwordSqlite { .. }
                | BibleSourceFormat::ObohuSqlite,
            ) => {
                let bytes = self
                    .provider
                    .fetch_bytes(url)
                    .await
                    .with_context(|| format!("failed to fetch bible archive from {}", url))?;
                build_ingestion_batch(&bytes, spec)
            }
            (
                BibleSource::LocalFile { env_var, .. },
                BibleSourceFormat::UsfmZip { .. }
                | BibleSourceFormat::MySwordSqlite { .. }
                | BibleSourceFormat::ObohuSqlite,
            ) => {
                let path = std::env::var(env_var).with_context(|| {
                    format!(
                        "environment variable {} must be set to the bible archive for {}",
                        env_var, spec.translation.code
                    )
                })?;
                let bytes = std::fs::read(&path).with_context(|| {
                    format!(
                        "failed to read bible archive for {} from {}",
                        spec.translation.code, path
                    )
                })?;
                build_ingestion_batch(&bytes, spec)
            }
        }
    }
}
