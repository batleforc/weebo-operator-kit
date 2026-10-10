//! Prometheus text exposition, as the operator serves it on
//! `WEEBO_METRICS_ADDR` (and the apiserver on `/metrics`): just enough to
//! sum counters and read histogram quantiles between two scrapes.

use std::collections::BTreeMap;

type Labels = BTreeMap<String, String>;

/// One scrape: every sample, keyed by metric name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scrape {
    samples: BTreeMap<String, Vec<(Labels, f64)>>,
}

impl Scrape {
    pub fn parse(text: &str) -> Self {
        let mut samples: BTreeMap<String, Vec<_>> = BTreeMap::new();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((name, labels, value)) = parse_line(line) {
                samples.entry(name).or_default().push((labels, value));
            }
        }
        Self { samples }
    }

    /// Sum of every series of `name` whose labels contain all of `matching`.
    pub fn sum(&self, name: &str, matching: &[(&str, &str)]) -> f64 {
        self.series(name, matching).map(|(_, v)| v).sum()
    }

    /// `value` of the first series of `name` matching, if any.
    pub fn value(&self, name: &str, matching: &[(&str, &str)]) -> Option<f64> {
        self.series(name, matching).map(|(_, v)| v).next()
    }

    /// Cumulative buckets of histogram `name` (`<name>_bucket`), summed over
    /// the series matching, by upper bound.
    pub fn buckets(&self, name: &str, matching: &[(&str, &str)]) -> Vec<(f64, f64)> {
        let mut by_le: BTreeMap<u64, (f64, f64)> = BTreeMap::new();
        for (labels, value) in self.series(&format!("{name}_bucket"), matching) {
            let Some(le) = labels.get("le").and_then(|l| parse_float(l)) else {
                continue;
            };
            by_le.entry(le.to_bits()).or_insert((le, 0.0)).1 += value;
        }
        let mut out: Vec<_> = by_le.into_values().collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    fn series<'a>(
        &'a self,
        name: &str,
        matching: &'a [(&str, &str)],
    ) -> impl Iterator<Item = (&'a BTreeMap<String, String>, f64)> + 'a {
        self.samples
            .get(name)
            .into_iter()
            .flatten()
            .filter(move |(labels, _)| {
                matching
                    .iter()
                    .all(|(k, v)| labels.get(*k).map(String::as_str) == Some(*v))
            })
            .map(|(l, v)| (l, *v))
    }
}

/// The `n` largest increases of counter `name` between two scrapes,
/// grouped by the values of `by` (`"LIST leases"`…), largest first.
pub fn top_deltas(
    before: &Scrape,
    after: &Scrape,
    name: &str,
    by: &[&str],
    n: usize,
) -> Vec<(String, f64)> {
    let group = |scrape: &Scrape| {
        let mut out: BTreeMap<String, f64> = BTreeMap::new();
        for (labels, value) in scrape.series(name, &[]) {
            let key = by
                .iter()
                .map(|k| labels.get(*k).map_or("", String::as_str))
                .collect::<Vec<_>>()
                .join(" ");
            *out.entry(key).or_default() += value;
        }
        out
    };
    let was = group(before);
    let mut deltas: Vec<(String, f64)> = group(after)
        .into_iter()
        .map(|(k, v)| {
            let d = v - was.get(&k).copied().unwrap_or(0.0);
            (k, d)
        })
        .filter(|(_, d)| *d > 0.0)
        .collect();
    deltas.sort_by(|a, b| b.1.total_cmp(&a.1));
    deltas.truncate(n);
    deltas
}

/// The `q` quantile (0..1) of what a histogram recorded between `before`
/// and `after`, interpolated linearly inside the bucket, Prometheus'
/// `histogram_quantile` way. `None` when nothing was recorded.
pub fn quantile_between(before: &[(f64, f64)], after: &[(f64, f64)], q: f64) -> Option<f64> {
    let delta: Vec<(f64, f64)> = after
        .iter()
        .map(|(le, count)| {
            let was = before
                .iter()
                .find(|(b, _)| b == le)
                .map_or(0.0, |(_, c)| *c);
            (*le, count - was)
        })
        .collect();
    let total = delta.last()?.1;
    if total <= 0.0 {
        return None;
    }
    let rank = q * total;
    let mut lower = (0.0, 0.0);
    for &(le, count) in &delta {
        if count >= rank {
            if le.is_infinite() {
                return Some(lower.0);
            }
            let in_bucket = count - lower.1;
            let frac = if in_bucket > 0.0 {
                (rank - lower.1) / in_bucket
            } else {
                1.0
            };
            return Some(lower.0 + (le - lower.0) * frac);
        }
        lower = (le, count);
    }
    Some(lower.0)
}

fn parse_line(line: &str) -> Option<(String, BTreeMap<String, String>, f64)> {
    let (head, value) = match line.find('}') {
        Some(end) => (&line[..=end], line[end + 1..].trim()),
        None => {
            let mut parts = line.splitn(2, char::is_whitespace);
            (parts.next()?, parts.next()?.trim())
        }
    };
    // A trailing timestamp is allowed after the value.
    let value = parse_float(value.split_whitespace().next()?)?;
    let (name, labels) = match head.find('{') {
        Some(start) => (
            &head[..start],
            parse_labels(&head[start + 1..head.len() - 1]),
        ),
        None => (head, BTreeMap::new()),
    };
    Some((name.to_string(), labels, value))
}

fn parse_labels(s: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut rest = s;
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim().trim_start_matches(',').trim().to_string();
        let after = &rest[eq + 1..];
        let Some(body) = after.strip_prefix('"') else {
            break;
        };
        let mut value = String::new();
        let mut chars = body.char_indices();
        let mut end = body.len();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => {
                    if let Some((_, e)) = chars.next() {
                        value.push(match e {
                            'n' => '\n',
                            other => other,
                        });
                    }
                }
                '"' => {
                    end = i + 1;
                    break;
                }
                c => value.push(c),
            }
        }
        out.insert(key, value);
        rest = &body[end..];
    }
    out
}

fn parse_float(s: &str) -> Option<f64> {
    match s {
        "+Inf" | "Inf" => Some(f64::INFINITY),
        "-Inf" => Some(f64::NEG_INFINITY),
        other => other.parse().ok(),
    }
}

/// `GET http://<addr>/metrics`, plain HTTP — the operator's own endpoint.
pub async fn scrape(addr: std::net::SocketAddr) -> std::io::Result<Scrape> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    stream
        .write_all(format!("GET /metrics HTTP/1.0\r\nHost: {addr}\r\n\r\n").as_bytes())
        .await?;
    let mut body = Vec::new();
    stream.read_to_end(&mut body).await?;
    let text = String::from_utf8_lossy(&body);
    let Some((head, payload)) = text.split_once("\r\n\r\n") else {
        return Err(std::io::Error::other("malformed HTTP response"));
    };
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return Err(std::io::Error::other(
            head.lines().next().unwrap_or_default().to_string(),
        ));
    }
    Ok(Scrape::parse(payload))
}

/// The apiserver's own `/metrics`, through `client` (an admin one: envtest's).
pub async fn scrape_apiserver(client: &kube::Client) -> Result<Scrape, kube::Error> {
    let request = http_request("/metrics");
    let text = client.request_text(request).await?;
    Ok(Scrape::parse(&text))
}

fn http_request(path: &str) -> http::Request<Vec<u8>> {
    http::Request::get(path)
        .body(Vec::new())
        .expect("a static GET request")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"
# HELP weebo_x_reconcile_total Reconcile runs
# TYPE weebo_x_reconcile_total counter
weebo_x_reconcile_total{kind="A",reason="Reconciled",result="success"} 10
weebo_x_reconcile_total{kind="B",reason="Boom",result="error"} 2
weebo_x_leader 1
weebo_x_reconcile_duration_seconds_bucket{kind="A",le="0.1"} 4
weebo_x_reconcile_duration_seconds_bucket{kind="A",le="1"} 8
weebo_x_reconcile_duration_seconds_bucket{kind="A",le="+Inf"} 10
weebo_x_reconcile_duration_seconds_bucket{kind="B",le="0.1"} 0
weebo_x_reconcile_duration_seconds_bucket{kind="B",le="1"} 2
weebo_x_reconcile_duration_seconds_bucket{kind="B",le="+Inf"} 2
weird{path="a \"quoted\", value"} 3 1700000000
"#;

    #[test]
    fn counters_and_gauges() {
        let s = Scrape::parse(TEXT);
        assert_eq!(s.sum("weebo_x_reconcile_total", &[]), 12.0);
        assert_eq!(
            s.sum("weebo_x_reconcile_total", &[("result", "error")]),
            2.0
        );
        assert_eq!(s.value("weebo_x_leader", &[]), Some(1.0));
        assert_eq!(
            s.value("weird", &[("path", "a \"quoted\", value")]),
            Some(3.0)
        );
        assert_eq!(s.sum("missing", &[]), 0.0);
    }

    #[test]
    fn top_deltas_group_and_rank() {
        let before = Scrape::parse(
            "r{verb=\"GET\",resource=\"leases\",code=\"200\"} 10\nr{verb=\"LIST\",resource=\"pods\"} 5",
        );
        let after = Scrape::parse(
            "r{verb=\"GET\",resource=\"leases\",code=\"200\"} 14\nr{verb=\"GET\",resource=\"leases\",code=\"409\"} 2\nr{verb=\"LIST\",resource=\"pods\"} 5\nr{verb=\"WATCH\",resource=\"pods\"} 1",
        );
        assert_eq!(
            top_deltas(&before, &after, "r", &["verb", "resource"], 8),
            vec![
                ("GET leases".to_string(), 6.0),
                ("WATCH pods".to_string(), 1.0)
            ]
        );
    }

    #[test]
    fn histogram_quantiles_over_a_delta() {
        let s = Scrape::parse(TEXT);
        let after = s.buckets("weebo_x_reconcile_duration_seconds", &[]);
        assert_eq!(after, vec![(0.1, 4.0), (1.0, 10.0), (f64::INFINITY, 12.0)]);
        // Everything since the start: the median (rank 6) is in (0.1, 1].
        let median = quantile_between(&[], &after, 0.5).unwrap();
        assert!((median - 0.4).abs() < 1e-9, "{median}");
        // Nothing new between two equal scrapes.
        assert_eq!(quantile_between(&after, &after, 0.5), None);
        // Only what came after `before`.
        let before = vec![(0.1, 4.0), (1.0, 4.0), (f64::INFINITY, 4.0)];
        let p = quantile_between(&before, &after, 0.5).unwrap();
        assert!(p > 0.1 && p <= 1.0);
    }
}
