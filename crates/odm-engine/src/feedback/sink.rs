//! Where a sent report goes: one form-encoded POST to a Google Form.
//!
//! A form is free and needs no infrastructure of ours; the unofficial
//! `formResponse` endpoint takes a plain POST. Everything form-specific is
//! the constants below and [`send`]'s body, so the sink can be swapped for a
//! Worker → GitHub Issues bridge later without touching anything else.

use super::item::Item;
use std::time::Duration;

/// The form's POST endpoint (its `/viewform` URL with the last segment
/// swapped). One field per question, keyed by the ids the form's own
/// pre-filled-link feature prints.
const FORM_URL: &str = "https://docs.google.com/forms/d/e/\
                        1FAIpQLScC9qO_DurasH-uqTzh7ld4JIh6Prr7U6_utCSb527lIohFTQ/formResponse";
const TITLE: &str = "entry.845689638";
const BODY: &str = "entry.74023224";
const HARNESS: &str = "entry.549086379";
const MODEL: &str = "entry.1905530866";
const PLATFORM: &str = "entry.67990661";
const BUILD: &str = "entry.994511765";

/// Points the sink somewhere else — a local listener in the test, or a dry
/// run that doesn't reach the real form.
const URL_ENV: &str = "ODM_FEEDBACK_URL";

/// Nothing should hang the user's Send button for longer than this.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Post one report. `Ok(())` means the sink took it (and the caller deletes
/// the file); the error is what the card shows under the failed Send.
pub fn send(item: &Item) -> Result<(), String> {
    send_to(&std::env::var(URL_ENV).unwrap_or_else(|_| FORM_URL.to_owned()), item)
}

/// The send itself, against a named endpoint — which is how the test posts
/// to a local listener without touching the process environment.
fn send_to(url: &str, item: &Item) -> Result<(), String> {
    let fields = [
        (TITLE, &item.title),
        (BODY, &item.body),
        (HARNESS, &item.harness),
        (MODEL, &item.model),
        (PLATFORM, &item.platform),
        (BUILD, &item.build),
    ];
    // Read the status ourselves: "the form said 400" is a better error than
    // ureq's own, and a redirect to the thank-you page is still a success.
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = config.into();
    let response = agent
        .post(url)
        .send_form(fields.map(|(k, v)| (k, v.as_str())))
        .map_err(|e| format!("could not reach the form: {e}"))?;
    let status = response.status().as_u16();
    match status {
        200..=299 => Ok(()),
        _ => Err(format!("the form answered {status}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// Serve one request, answer `status`, and hand back what was posted.
    fn one_request(listener: TcpListener, status: u16) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("a request");
            let mut reader = BufReader::new(stream);
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse().unwrap();
                }
                if line.trim().is_empty() {
                    break;
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).unwrap();
            let reason = if status == 200 { "OK" } else { "Bad Request" };
            let mut stream = reader.into_inner();
            write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\n\r\n").unwrap();
            stream.flush().unwrap();
            String::from_utf8(body).unwrap()
        })
    }

    fn item() -> Item {
        let mut item = Item::new(
            "it crashed".into(),
            "line one\nline two — café".into(),
            "Claude Code".into(),
            "opus".into(),
        );
        item.platform = "linux x86_64".into();
        item.build = "odm 0.1.0 (abc1234, x86_64-unknown-linux-gnu)".into();
        item
    }

    /// The POST carries every field, encoded as a form — newlines and
    /// non-ASCII included, since that is what a bug report is made of.
    #[test]
    fn a_send_posts_every_field() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/formResponse", listener.local_addr().unwrap());
        let served = one_request(listener, 200);
        send_to(&url, &item()).expect("200 means recorded");

        let body = served.join().unwrap();
        let pairs: std::collections::HashMap<&str, String> = body
            .split('&')
            .map(|p| {
                let (k, v) = p.split_once('=').expect("k=v");
                (k, percent_decode(v))
            })
            .collect();
        assert_eq!(pairs[TITLE], "it crashed");
        assert_eq!(pairs[BODY], "line one\nline two — café");
        assert_eq!(pairs[HARNESS], "Claude Code");
        assert_eq!(pairs[MODEL], "opus");
        assert_eq!(pairs[PLATFORM], "linux x86_64");
        assert_eq!(pairs[BUILD], "odm 0.1.0 (abc1234, x86_64-unknown-linux-gnu)");
        assert_eq!(pairs.len(), 6, "no field is invented or dropped: {body}");
    }

    /// Anything but a 2xx keeps the item — the user sees why and can retry.
    #[test]
    fn a_refused_send_reports_the_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/formResponse", listener.local_addr().unwrap());
        let served = one_request(listener, 400);
        let result = send_to(&url, &item());
        let _ = served.join();
        let error = result.expect_err("a refused send is an error");
        assert!(error.contains("400"), "{error}");
    }

    fn percent_decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'+' => out.push(b' '),
                b'%' => {
                    let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                    out.push(u8::from_str_radix(hex, 16).unwrap());
                    i += 2;
                }
                b => out.push(b),
            }
            i += 1;
        }
        String::from_utf8(out).unwrap()
    }
}
