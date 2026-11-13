//! The group's bucket over S3's API (Cloudflare R2, or anything that speaks it and enforces
//! conditional writes), with Signature Version 4 worked out here: path-style URLs, the payload's
//! SHA-256 signed, no SDK.
//!
//! A request that fails on the way, or with a server error, is tried again; one the storage
//! refuses is not. A device whose clock is off gets one correction from the storage's own
//! `Date` header, since a signature more than 15 minutes out is rejected.
use crate::{
    Error, Result,
    codec::{hex, hmac_sha256, sha256},
    store::{Condition, Etag, Listed, Object, Store},
};
use std::{
    io::Read,
    sync::atomic::{AtomicI64, Ordering},
    time::Duration,
};

/// Where a world's bucket is and the key that opens it.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Bucket {
    /// `https://<account>.r2.cloudflarestorage.com`.
    pub endpoint: String,
    pub bucket: String,
    /// `auto` for R2.
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
}

impl std::fmt::Debug for Bucket {
    /// Without the secret: this ends up in logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bucket")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}

const TRIES: u32 = 4;
/// The largest answer read into memory: a pack's range or a file list.
const MAX_BODY: u64 = 64 << 20;
const SIGNED: &str = "host;x-amz-content-sha256;x-amz-date";

pub struct S3Store {
    agent: ureq::Agent,
    /// `scheme://host[:port]`.
    origin: String,
    /// The `Host` header as the request sends it.
    host: String,
    bucket: String,
    region: String,
    access_key: String,
    secret_key: String,
    /// Seconds to add to this device's clock, learnt from the storage.
    skew: AtomicI64,
}

/// RFC 3986 percent-encoding as S3 signs it: everything but the unreserved characters, and
/// `/` kept in a path.
fn encode(text: &str, path: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            b'/' if path => out.push('/'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The query as S3 signs and receives it: sorted by name, both sides encoded.
fn canonical_query(query: &[(&str, &str)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(name, value)| (encode(name, false), encode(value, false)))
        .collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// What one request is signed over.
struct Signing<'a> {
    method: &'a str,
    /// Already encoded.
    path: &'a str,
    /// Already canonical.
    query: &'a str,
    /// Lower-case names in order, each with its value.
    headers: &'a [(&'a str, &'a str)],
    payload_hash: &'a str,
    /// `20130524T000000Z`.
    time: &'a str,
    region: &'a str,
    access_key: &'a str,
    secret_key: &'a str,
}

/// The `Authorization` header (AWS Signature Version 4, service `s3`).
fn authorization(s: &Signing<'_>) -> String {
    let date = &s.time[..8];
    let names: Vec<&str> = s.headers.iter().map(|(name, _)| *name).collect();
    let signed = names.join(";");
    let headers: String = s
        .headers
        .iter()
        .map(|(name, value)| format!("{name}:{}\n", value.trim()))
        .collect();
    let request = format!(
        "{}\n{}\n{}\n{headers}\n{signed}\n{}",
        s.method, s.path, s.query, s.payload_hash
    );
    let scope = format!("{date}/{}/s3/aws4_request", s.region);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        s.time,
        hex(&sha256(request.as_bytes()))
    );
    let mut key = hmac_sha256(format!("AWS4{}", s.secret_key).as_bytes(), date.as_bytes());
    for part in [s.region, "s3", "aws4_request"] {
        key = hmac_sha256(&key, part.as_bytes());
    }
    let signature = hex(&hmac_sha256(&key, to_sign.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope},SignedHeaders={signed},Signature={signature}",
        s.access_key
    )
}

/// The text between `<name>` and `</name>` in `xml` from `from` on, and where it ends.
fn element<'a>(xml: &'a str, name: &str, from: usize) -> Option<(&'a str, usize)> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml[from..].find(&open)? + from + open.len();
    let end = xml[start..].find(&close)? + start;
    Some((&xml[start..end], end + close.len()))
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// One page of a ListObjectsV2 answer: its objects, and the token of the next page.
fn parse_listing(xml: &str) -> Result<(Vec<Listed>, Option<String>)> {
    let mut listed = Vec::new();
    let mut at = 0;
    while let Some((contents, next)) = element(xml, "Contents", at) {
        let key = element(contents, "Key", 0).map(|(key, _)| unescape(key));
        let size = element(contents, "Size", 0).and_then(|(size, _)| size.trim().parse().ok());
        match (key, size) {
            (Some(key), Some(size)) => listed.push(Listed { key, size }),
            _ => return Err(Error::Format("a listing entry without a key or size".into())),
        }
        at = next;
    }
    let more = element(xml, "IsTruncated", 0).is_some_and(|(v, _)| v.trim() == "true");
    let token = element(xml, "NextContinuationToken", 0).map(|(token, _)| unescape(token));
    match (more, token) {
        (true, None) => Err(Error::Format("a cut listing without its next page".into())),
        (true, token) => Ok((listed, token)),
        (false, _) => Ok((listed, None)),
    }
}

/// How a request ended when it didn't end well.
enum Failure {
    /// The storage answered with this status and body.
    Status(u16, String),
    /// No answer.
    Network(String),
}

impl S3Store {
    pub fn new(config: &Bucket) -> Result<Self> {
        let origin = config.endpoint.trim().trim_end_matches('/').to_owned();
        let host = match origin.split_once("://") {
            Some(("https", host)) => host,
            // A bucket on this machine (tests against a local server); anything else in clear
            // text would send the key over the network.
            Some(("http", host)) if host.starts_with("127.0.0.1") || host.starts_with("localhost") => host,
            _ => return Err(Error::Format("the storage address must start with https://".into())),
        };
        if host.is_empty() || host.contains(['/', '?', '#', '@', ' ']) {
            return Err(Error::Format("the storage address is a host name only".into()));
        }
        for (what, value) in [
            ("bucket name", &config.bucket),
            ("region", &config.region),
            ("access key", &config.access_key),
            ("secret key", &config.secret_key),
        ] {
            if value.trim().is_empty() || value.contains(['\n', '\r']) {
                return Err(Error::Format(format!("the {what} is missing")));
            }
        }
        Ok(Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(10))
                .timeout_read(Duration::from_secs(30))
                .timeout_write(Duration::from_secs(30))
                .user_agent("horizon-shared-worlds")
                .build(),
            host: host.to_owned(),
            origin,
            bucket: config.bucket.trim().to_owned(),
            region: config.region.trim().to_owned(),
            access_key: config.access_key.trim().to_owned(),
            secret_key: config.secret_key.trim().to_owned(),
            skew: AtomicI64::new(0),
        })
    }

    /// One signed request, sent once.
    fn send_once(
        &self,
        method: &str,
        key: Option<&str>,
        query: &[(&str, &str)],
        extra: &[(&str, String)],
        body: &[u8],
    ) -> std::result::Result<ureq::Response, Failure> {
        let mut path = format!("/{}", encode(&self.bucket, false));
        if let Some(key) = key {
            path.push('/');
            path.push_str(&encode(key, true));
        }
        let query = canonical_query(query);
        let now = chrono::Utc::now() + chrono::Duration::seconds(self.skew.load(Ordering::Relaxed));
        let time = now.format("%Y%m%dT%H%M%SZ").to_string();
        let payload_hash = hex(&sha256(body));
        let authorization = authorization(&Signing {
            method,
            path: &path,
            query: &query,
            headers: &[
                ("host", self.host.as_str()),
                ("x-amz-content-sha256", payload_hash.as_str()),
                ("x-amz-date", time.as_str()),
            ],
            payload_hash: &payload_hash,
            time: &time,
            region: &self.region,
            access_key: &self.access_key,
            secret_key: &self.secret_key,
        });
        debug_assert!(authorization.contains(SIGNED));
        let mut url = format!("{}{path}", self.origin);
        if !query.is_empty() {
            url.push('?');
            url.push_str(&query);
        }
        let mut request = self
            .agent
            .request(method, &url)
            .set("x-amz-date", &time)
            .set("x-amz-content-sha256", &payload_hash)
            .set("Authorization", &authorization);
        for (name, value) in extra {
            request = request.set(name, value);
        }
        let answer = if method == "PUT" {
            request.send_bytes(body)
        } else {
            request.call()
        };
        match answer {
            Ok(response) => Ok(response),
            Err(ureq::Error::Status(status, response)) => {
                let date = response.header("Date").map(str::to_owned);
                let body = response.into_string().unwrap_or_default();
                if status == 403 && body.contains("RequestTimeTooSkewed") {
                    self.correct_clock(date.as_deref());
                }
                Err(Failure::Status(status, body))
            }
            Err(ureq::Error::Transport(e)) => Err(Failure::Network(e.to_string())),
        }
    }

    /// The storage says this device's clock is too far off: follow the storage's.
    fn correct_clock(&self, date: Option<&str>) {
        let Some(theirs) = date.and_then(|d| chrono::DateTime::parse_from_rfc2822(d).ok()) else {
            return;
        };
        let skew = theirs.timestamp() - chrono::Utc::now().timestamp();
        log::warn!("Shared worlds: this device's clock is {skew} s off the storage's; following the storage");
        self.skew.store(skew, Ordering::Relaxed);
    }

    /// A signed request, tried again while trying again can help. `Ok(Err(status))` is an answer
    /// the caller asked to be told about (`expected`), not a failure.
    fn send(
        &self,
        method: &str,
        key: Option<&str>,
        query: &[(&str, &str)],
        extra: &[(&str, String)],
        body: &[u8],
        expected: &[u16],
    ) -> Result<std::result::Result<ureq::Response, u16>> {
        let mut last = String::new();
        for attempt in 0..TRIES {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(250 << (2 * (attempt - 1))));
            }
            match self.send_once(method, key, query, extra, body) {
                Ok(response) => return Ok(Ok(response)),
                Err(Failure::Status(status, _)) if expected.contains(&status) => return Ok(Err(status)),
                Err(Failure::Status(status, body)) => {
                    let skewed = status == 403 && body.contains("RequestTimeTooSkewed");
                    last = format!("{method} answered {status}: {}", summary(&body));
                    if !(skewed || status == 429 || status >= 500) {
                        break;
                    }
                }
                Err(Failure::Network(e)) => last = format!("{method} failed: {e}"),
            }
        }
        Err(Error::Store(last))
    }
}

/// An error body's code and message, without the rest of the XML.
fn summary(body: &str) -> String {
    let code = element(body, "Code", 0).map_or("", |(code, _)| code);
    let message = element(body, "Message", 0).map_or("", |(message, _)| message);
    if code.is_empty() && message.is_empty() {
        body.chars().take(200).collect()
    } else {
        format!("{code} {message}")
    }
}

fn read_body(response: ureq::Response) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    response
        .into_reader()
        .take(MAX_BODY + 1)
        .read_to_end(&mut data)
        .map_err(|e| Error::Store(format!("the answer was cut off: {e}")))?;
    if data.len() as u64 > MAX_BODY {
        return Err(Error::Corrupt("an answer larger than any object of ours".into()));
    }
    Ok(data)
}

fn etag_of(response: &ureq::Response) -> Result<Etag> {
    response
        .header("ETag")
        .map(|tag| Etag(tag.to_owned()))
        .ok_or_else(|| Error::Store("the storage gave the object no tag".into()))
}

impl Store for S3Store {
    fn get(&self, key: &str) -> Result<Option<Object>> {
        match self.send("GET", Some(key), &[], &[], &[], &[404])? {
            Ok(response) => {
                let etag = etag_of(&response)?;
                Ok(Some(Object {
                    data: read_body(response)?,
                    etag,
                }))
            }
            Err(_) => Ok(None),
        }
    }

    fn get_range(&self, key: &str, start: u64, length: u64) -> Result<Vec<u8>> {
        if length == 0 {
            return Ok(Vec::new());
        }
        let range = format!("bytes={start}-{}", start + length - 1);
        let response = self
            .send("GET", Some(key), &[], &[("Range", range)], &[], &[])?
            .map_err(|status| Error::Store(format!("GET answered {status}")))?;
        // 206 is the range; a storage that ignores the header sends the whole object as 200.
        let whole = response.status() == 200;
        let mut data = read_body(response)?;
        if whole {
            let end = start.checked_add(length).filter(|end| *end <= data.len() as u64);
            let Some(end) = end else {
                return Err(Error::Corrupt(format!("{key} is shorter than {start}+{length}")));
            };
            data = data[start as usize..end as usize].to_vec();
        }
        if data.len() as u64 != length {
            return Err(Error::Corrupt(format!(
                "{key}: {} bytes came for a range of {length}",
                data.len()
            )));
        }
        Ok(data)
    }

    fn put(&self, key: &str, data: &[u8], condition: &Condition) -> Result<Etag> {
        let extra = match condition {
            Condition::Always => vec![],
            Condition::Absent => vec![("If-None-Match", "*".to_owned())],
            Condition::Unchanged(etag) => vec![("If-Match", etag.0.clone())],
        };
        // 412: the condition doesn't hold. 409: another conditional write to the key is under
        // way, which for this caller is the same answer: read it again.
        match self.send("PUT", Some(key), &[], &extra, data, &[412, 409])? {
            Ok(response) => etag_of(&response),
            Err(_) => Err(Error::Refused),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<Listed>> {
        let mut listed = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![("list-type", "2"), ("prefix", prefix)];
            if let Some(token) = &token {
                query.push(("continuation-token", token));
            }
            let response = self
                .send("GET", None, &query, &[], &[], &[])?
                .map_err(|status| Error::Store(format!("the listing answered {status}")))?;
            let body = read_body(response)?;
            let xml = std::str::from_utf8(&body).map_err(|_| Error::Format("a listing that isn't text".into()))?;
            let (page, next) = parse_listing(xml)?;
            listed.extend(page);
            match next {
                Some(next) => token = Some(next),
                None => break,
            }
        }
        listed.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(listed)
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.send("DELETE", Some(key), &[], &[], &[], &[404]).map(|_| ())
    }
}

/// Proves on the bucket itself what a shared world relies on, with a throwaway object: a second
/// "only if absent" write is refused, a write on a stale tag is refused, a range comes back as
/// asked, the listing sees the object. A bucket that fails any of these can't hold a shared world.
pub fn self_test(store: &dyn Store, prefix: &str) -> Result<()> {
    let key = format!("{prefix}selftest/{}", hex(&crate::codec::random::<8>()?));
    let outcome = self_test_on(store, &key);
    let removed = store.delete(&key);
    outcome.and(removed)
}

fn self_test_on(store: &dyn Store, key: &str) -> Result<()> {
    let wrong = |what: &str| Error::Store(format!("this storage can't hold a shared world: {what}"));
    {
        let first = store.put(key, b"0123456789", &Condition::Absent)?;
        match store.put(key, b"second", &Condition::Absent) {
            Err(Error::Refused) => {}
            Ok(_) => return Err(wrong("it let a second writer replace a new record")),
            Err(e) => return Err(e),
        }
        let second = store.put(key, b"abcdefghij", &Condition::Unchanged(first.clone()))?;
        match store.put(key, b"stale", &Condition::Unchanged(first)) {
            Err(Error::Refused) => {}
            Ok(_) => return Err(wrong("it let a writer with an old copy replace a newer record")),
            Err(e) => return Err(e),
        }
        let object = store.get(key)?.ok_or_else(|| wrong("a record it accepted is gone"))?;
        if object.data != b"abcdefghij" || object.etag != second {
            return Err(wrong("it returned a different record than it accepted"));
        }
        if store.get_range(key, 3, 4)? != b"defg" {
            return Err(wrong("it returned the wrong part of a file"));
        }
        if !store.list(key)?.iter().any(|o| o.key == key && o.size == 10) {
            return Err(wrong("its listing misses a record it holds"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn example<'a>(
        path: &'a str,
        query: &'a str,
        headers: &'a [(&'a str, &'a str)],
    ) -> Signing<'a> {
        Signing {
            method: "GET",
            path,
            query,
            headers,
            payload_hash: EMPTY,
            time: "20130524T000000Z",
            region: "us-east-1",
            access_key: "AKIAIOSFODNN7EXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        }
    }

    /// Amazon's own worked examples ("Signature Calculations for the Authorization Header").
    #[test]
    fn signatures_match_amazons_examples() {
        let get = authorization(&example(
            "/test.txt",
            "",
            &[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("range", "bytes=0-9"),
                ("x-amz-content-sha256", EMPTY),
                ("x-amz-date", "20130524T000000Z"),
            ],
        ));
        assert!(
            get.ends_with("Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"),
            "{get}"
        );
        assert!(get.contains("Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,"));
        assert!(get.contains("SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,"));
        let list = authorization(&example(
            "/",
            &canonical_query(&[("prefix", "J"), ("max-keys", "2")]),
            &[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("x-amz-content-sha256", EMPTY),
                ("x-amz-date", "20130524T000000Z"),
            ],
        ));
        assert!(
            list.ends_with("Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"),
            "{list}"
        );
    }

    #[test]
    fn keys_and_queries_are_encoded_as_s3_signs_them() {
        assert_eq!(encode("w/ab12/gen/0001-02.json", true), "w/ab12/gen/0001-02.json");
        assert_eq!(encode("a b/c+d~e", true), "a%20b/c%2Bd~e");
        assert_eq!(encode("w/ab/", false), "w%2Fab%2F");
        assert_eq!(
            canonical_query(&[("prefix", "w/a b/"), ("list-type", "2"), ("continuation-token", "x=/+")]),
            "continuation-token=x%3D%2F%2B&list-type=2&prefix=w%2Fa%20b%2F"
        );
    }

    #[test]
    fn listings_are_read_page_by_page() {
        let page = "<?xml version=\"1.0\"?><ListBucketResult><IsTruncated>true</IsTruncated>\
            <Contents><Key>w/a/gen/1.json</Key><LastModified>x</LastModified><Size>120</Size></Contents>\
            <Contents><Key>w/a/p&amp;q</Key><Size>7</Size></Contents>\
            <NextContinuationToken>tok&lt;1</NextContinuationToken></ListBucketResult>";
        let (listed, next) = parse_listing(page).unwrap();
        assert_eq!(
            listed,
            [
                Listed { key: "w/a/gen/1.json".into(), size: 120 },
                Listed { key: "w/a/p&q".into(), size: 7 }
            ]
        );
        assert_eq!(next.as_deref(), Some("tok<1"));
        let last = "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>";
        assert_eq!(parse_listing(last).unwrap(), (vec![], None));
        assert!(parse_listing("<ListBucketResult><IsTruncated>true</IsTruncated></ListBucketResult>").is_err());
        assert!(parse_listing("<Contents><Key>k</Key></Contents>").is_err());
    }

    #[test]
    fn only_https_or_this_machine_is_accepted() {
        let bucket = |endpoint: &str| Bucket {
            endpoint: endpoint.into(),
            bucket: "b".into(),
            region: "auto".into(),
            access_key: "k".into(),
            secret_key: "s".into(),
        };
        assert!(S3Store::new(&bucket("https://abc.r2.cloudflarestorage.com/")).is_ok());
        assert!(S3Store::new(&bucket("http://127.0.0.1:9000")).is_ok());
        assert!(S3Store::new(&bucket("http://example.com")).is_err());
        assert!(S3Store::new(&bucket("abc.r2.cloudflarestorage.com")).is_err());
        assert!(S3Store::new(&bucket("https://host/path")).is_err());
        let mut missing = bucket("https://h");
        missing.secret_key = " ".into();
        assert!(S3Store::new(&missing).is_err());
        assert!(!format!("{:?}", bucket("https://h")).contains("secret"));
    }

    #[test]
    fn the_self_test_passes_a_store_that_enforces_conditions() {
        let store = MemoryStore::new();
        self_test(&store, "w/x/").unwrap();
        assert!(store.keys().is_empty(), "the throwaway object is removed");
    }

    /// A store that takes every write, as storage without conditional writes does.
    struct Lax(MemoryStore);
    impl Store for Lax {
        fn get(&self, key: &str) -> Result<Option<Object>> {
            self.0.get(key)
        }
        fn get_range(&self, key: &str, start: u64, length: u64) -> Result<Vec<u8>> {
            self.0.get_range(key, start, length)
        }
        fn put(&self, key: &str, data: &[u8], _: &Condition) -> Result<Etag> {
            self.0.put(key, data, &Condition::Always)
        }
        fn list(&self, prefix: &str) -> Result<Vec<Listed>> {
            self.0.list(prefix)
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.0.delete(key)
        }
    }

    #[test]
    fn the_self_test_refuses_a_store_that_ignores_conditions() {
        let store = Lax(MemoryStore::new());
        let error = self_test(&store, "").unwrap_err().to_string();
        assert!(error.contains("second writer"), "{error}");
        assert!(store.0.keys().is_empty());
    }
}
