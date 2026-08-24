use std::{
    collections::HashMap,
    sync::{atomic::AtomicU32, Arc, RwLock},
};

use once_cell::sync::Lazy;
use rand::Rng;
use regex::Regex;
use reqwest::{header, Client};
use time::OffsetDateTime;

use crate::{
    client::{PoToken, YOUTUBE_HOME_URL, YOUTUBE_MUSIC_HOME_URL},
    error::{Error, ExtractionError},
    util,
};

/// Límite de saltos de redirect por endpoint.
const REDIRECT_LIMIT: usize = 6;

/// Cookies de consentimiento para evitar el rebote a consent.youtube.com en
/// regiones que exigen aviso. `SOCS=CAISAiAD` (consent aceptado) más
/// `CONSENT=YES+...` (el formato clásico que yt-dlp usa desde 2021).
const CONSENT_COOKIES: &str = "SOCS=CAISAiAD; CONSENT=YES+cb.20210328-17-p0.en+FX+000";

/// Origen (`https://<host>`) de una URL, para Origin/Referer por petición.
fn host_origin(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(u) => u
            .host_str()
            .map(|h| format!("https://{h}"))
            .unwrap_or_else(|| "https://www.youtube.com".to_string()),
        Err(_) => "https://www.youtube.com".to_string(),
    }
}

/// To increase privacy and possibly circumvent rate limits, RustyPipe uses multiple
/// visitor data IDs. These are held in this cache object.
///
/// On instantiation, the cache is empty, so for the first requests new visitor data IDs
/// have to be requested. For subsequent requests a random ID from the cache is picked.
/// After req_limit requests, a new token is requested asynchronously and added to the cache
/// to prevent the IDs from being overused.
///
/// The cache's maximum size is limited. If more IDs are added, the oldest ones are evicted.
#[derive(Clone)]
pub struct VisitorDataCache {
    inner: Arc<VisitorDataCacheRef>,
}

struct VisitorDataCacheRef {
    req_counter: AtomicU32,
    visitor_data: RwLock<Vec<String>>,
    session_potoken: RwLock<HashMap<String, PoToken>>,
    http: Client,
    /// Number of requests after which a new token is requested
    req_limit: u32,
    /// Maximum size of the cache
    max_size: usize,
}

static VISITOR_DATA_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#""visitorData":"([\w\d_\-%]+?)""#).unwrap());

impl VisitorDataCache {
    pub fn new(http: Client, req_limit: u32, max_size: usize) -> Self {
        Self {
            inner: VisitorDataCacheRef {
                req_counter: Default::default(),
                visitor_data: Default::default(),
                session_potoken: Default::default(),
                http,
                req_limit,
                max_size: max_size - 1,
            }
            .into(),
        }
    }

    /// Fetch a new visitor data ID from YouTube
    async fn fetch_visitor_data(&self) -> Result<String, Error> {
        tracing::debug!("getting YT visitor data");
        // YouTube ya no sirve `__Secure-YEC` (cookie de borrado vacía) y
        // `music.youtube.com` ha dejado de incluir `visitorData` en su HTML;
        // además redirige (302) según región/consent. Se prueba en orden y se
        // acepta el primer endpoint que dé un id válido.
        let mut last_err = None;
        for url in [YOUTUBE_MUSIC_HOME_URL, YOUTUBE_HOME_URL] {
            match self.fetch_visitor_data_from(url).await {
                Ok(vd) => {
                    tracing::debug!("visitor data {vd} obtenido de {url}");
                    return Ok(vd);
                }
                Err(e) => {
                    tracing::debug!("visitor data: {url} falló: {e}");
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            Error::Extraction(ExtractionError::InvalidData(
                "Could not get visitor data from any endpoint".into(),
            ))
        }))
    }

    /// Intenta extraer visitor data de un endpoint concreto, siguiendo los
    /// redirects a mano (el cliente compartido usa `Policy::none()`).
    async fn fetch_visitor_data_from(&self, start_url: &str) -> Result<String, Error> {
        let mut url = start_url.to_owned();
        for _ in 0..REDIRECT_LIMIT {
            // Origin/Referer del host de la petición actual: seguirlos fijos al
            // primer host provoca rebotes de consent al cambiar de dominio.
            let origin = host_origin(&url);
            let resp = self
                .inner
                .http
                .get(&url)
                .header(header::ORIGIN, origin.clone())
                .header(header::REFERER, origin.clone())
                .header(header::COOKIE, CONSENT_COOKIES)
                .send()
                .await?;

            let vdata = resp
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .find_map(|c| {
                    if let Ok(cookie) = c.to_str() {
                        if let Some(after) = cookie.strip_prefix("__Secure-YEC=") {
                            return after
                                .split_once(';')
                                .map(|s| s.0.to_owned())
                                .filter(|s| !s.is_empty());
                        }
                    }
                    None
                });
            if let Some(vdata) = vdata {
                return Ok(vdata);
            }

            // Redirect sin cookie: seguir el `Location` hasta un límite.
            if resp.status().is_redirection() {
                if let Some(location) = resp
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|l| l.to_str().ok())
                {
                    url = if location.starts_with('/') {
                        format!("{origin}{location}")
                    } else {
                        location.to_owned()
                    };
                    tracing::debug!("visitor data: siguiendo redirect a {url}");
                    continue;
                }
            }

            if resp.status().is_success() {
                let html = resp.text().await?;
                if let Some(vd) = util::get_cg_from_regex(&VISITOR_DATA_REGEX, &html, 1) {
                    return Ok(vd);
                }
                return Err(Error::Extraction(ExtractionError::InvalidData(
                    "Could not find visitor data on html page".into(),
                )));
            }

            return Err(Error::Extraction(ExtractionError::InvalidData(
                format!("Could not get visitor data, status: {}", resp.status()).into(),
            )));
        }

        Err(Error::Extraction(ExtractionError::InvalidData(
            "Could not get visitor data, too many redirects".into(),
        )))
    }

    /// Fetch a new visitor data ID and store it in the cache
    pub async fn new_visitor_data(&self) -> Result<String, Error> {
        let vd = self.fetch_visitor_data().await?;

        self.inner
            .req_counter
            .store(0, std::sync::atomic::Ordering::Relaxed);
        let mut vds = self.inner.visitor_data.write().unwrap();
        for _ in 0..(vds.len().saturating_sub(self.inner.max_size)) {
            let rem = vds.remove(0);
            {
                let mut pots = self.inner.session_potoken.write().unwrap();
                pots.remove(&rem);
            }
            tracing::debug!("visitor data {rem} removed from cache");
        }
        vds.push(vd.to_owned());
        tracing::debug!("visitor data {} added to cache ({} ids)", vd, vds.len());
        Ok(vd)
    }

    /// Get a visitor data ID from the cache
    pub async fn get(&self) -> Result<String, Error> {
        // Request a new visitor data ID in the background after a set number of requests
        if self
            .inner
            .req_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            >= self.inner.req_limit
        {
            self.inner
                .req_counter
                .store(0, std::sync::atomic::Ordering::Relaxed);
            let nc = self.clone();
            tokio::spawn(async move {
                // El refresco en segundo plano nunca debe panickear: un fallo
                // de red (302, timeout, anti-bot) se loguea y se reintenta en
                // la siguiente llamada.
                if let Err(e) = nc.new_visitor_data().await {
                    tracing::debug!("visitor data refresh failed: {e}");
                }
            });
        }

        {
            let vds = self.inner.visitor_data.read().unwrap();
            if !vds.is_empty() {
                let mut rng = rand::rng();
                let vd = vds[rng.random_range(0..vds.len())].to_owned();
                tracing::debug!("visitor data {vd} picked from cache");
                return Ok(vd);
            }
        }
        // Fetch new visitor data if the cache is empty
        self.new_visitor_data().await
    }

    /// Remove a visitor data ID from the cache.
    ///
    /// This also removes the PO token associated with that ID.
    pub fn remove(&self, visitor_data: &str) {
        let mut vds = self.inner.visitor_data.write().unwrap();
        if let Some(i) = vds.iter().position(|x| x == visitor_data) {
            vds.remove(i);
            let mut pots = self.inner.session_potoken.write().unwrap();
            pots.remove(visitor_data);
            tracing::debug!("visitor data {visitor_data} removed from cache");
        }
    }

    /// Store a session PO token in the cache
    pub fn store_pot(&self, visitor_data: &str, po_token: PoToken) {
        let mut pots = self.inner.session_potoken.write().unwrap();
        pots.insert(visitor_data.to_owned(), po_token);
    }

    /// Get a session PO token from the cache
    pub fn get_pot(&self, visitor_data: &str) -> Option<PoToken> {
        let pots = self.inner.session_potoken.read().unwrap();
        if let Some(entry) = pots.get(visitor_data) {
            if entry.valid_until > OffsetDateTime::now_utc() + time::Duration::minutes(10) {
                return Some(entry.clone());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::client::DEFAULT_UA;

    use super::*;

    use tracing_test::traced_test;

    #[tokio::test]
    #[traced_test]
    async fn get_visitor_data() {
        let cache = VisitorDataCache::new(
            Client::builder().user_agent(DEFAULT_UA).build().unwrap(),
            2,
            2,
        );
        // Get initial visitor data
        let v1 = cache.get().await.unwrap();

        // Run as many request as necessary to fetch second visitor data
        for _ in 0..=cache.inner.req_limit {
            let got = cache.get().await.unwrap();
            assert_eq!(got, v1);
        }

        // Second visitor data does not arrive instantly, request immediately after returns the first data
        let vds_len = cache.inner.visitor_data.read().unwrap().len();
        assert_eq!(vds_len, 1);

        // Wait for the second visitor data to arrive
        tokio::time::sleep(Duration::from_millis(1000)).await;
        let vds_len = cache.inner.visitor_data.read().unwrap().len();
        assert_eq!(vds_len, 2);
    }

    #[tokio::test]
    #[traced_test]
    async fn cache_potoken() {
        let cache = VisitorDataCache::new(
            Client::builder().user_agent(DEFAULT_UA).build().unwrap(),
            1,
            2,
        );
        let v1 = cache.get().await.unwrap();
        let pot1 = PoToken {
            po_token: "pot1".to_owned(),
            valid_until: OffsetDateTime::now_utc() + time::Duration::hours(1),
        };
        cache.store_pot(&v1, pot1.clone());
        assert_eq!(cache.get_pot(&v1).unwrap(), pot1);

        for _ in 0..4 {
            cache.get().await.unwrap();
        }

        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(1000)).await;
            {
                let vd = cache.inner.visitor_data.read().unwrap();
                if !vd.contains(&v1) {
                    break;
                }
            }
        }
        {
            let vd = cache.inner.visitor_data.read().unwrap();
            assert!(!vd.contains(&v1), "first token still present");
        }

        assert_eq!(cache.get_pot(&v1), None);
    }
}
