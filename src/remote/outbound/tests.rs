use std::time::Duration;

use super::{read_bounded_body, BodyError};

/// A listener answering its first request with `body` and every later one
/// with nothing. A body built from bytes declares its length; one built
/// from a stream is chunked and declares none.
pub(crate) async fn answering(body: axum::body::Body) -> (String, tokio::task::JoinHandle<()>) {
    let body = std::sync::Arc::new(std::sync::Mutex::new(Some(body)));
    let router = axum::Router::new().fallback(move || {
        let body = std::sync::Arc::clone(&body);
        async move {
            let body = body
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(axum::body::Body::empty);
            axum::response::Response::builder()
                .status(200)
                .header("content-type", "application/json")
                .body(body)
                .unwrap()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{address}/"), server)
}

/// A body that never ends: the same block of spaces, forever.
pub(crate) fn endless(prefix: &'static str) -> axum::body::Body {
    let head = futures_util::stream::once(async move {
        Ok::<_, std::io::Error>(bytes::Bytes::from_static(prefix.as_bytes()))
    });
    let tail = futures_util::stream::repeat_with(|| {
        Ok::<_, std::io::Error>(bytes::Bytes::from(vec![b' '; 64 * 1024]))
    });
    axum::body::Body::from_stream(futures_util::StreamExt::chain(head, tail))
}

async fn read(body: axum::body::Body, limit: usize) -> Result<Vec<u8>, BodyError> {
    let (url, server) = answering(body).await;
    let client = super::client(Duration::from_secs(5)).unwrap();
    let response = client.get(url).send().await.unwrap();
    // Bounded by the test, not by the reader: without the ceiling, the
    // endless body below is read until memory runs out, and this is what
    // turns that into a failure with a reason.
    let read = tokio::time::timeout(Duration::from_secs(20), read_bounded_body(response, limit))
        .await
        .expect("a bounded read must end on its own");
    server.abort();
    read
}

#[tokio::test]
async fn an_answer_within_the_ceiling_is_read_whole() {
    let body = read(axum::body::Body::from("{\"ok\":true}"), 1024).await;
    assert_eq!(body.unwrap(), b"{\"ok\":true}");
}

/// The case `Content-Length` alone cannot catch: a chunked body that
/// never says how long it is, and never stops.
#[tokio::test]
async fn an_endless_answer_is_cut_off_at_the_ceiling() {
    let body = read(endless("{\"status\":\"failure\""), 1024 * 1024).await;
    assert_eq!(body, Err(BodyError::TooLarge));
}

/// A chunked answer that ends, but past the ceiling: the running count,
/// not the header, is what refuses it.
#[tokio::test]
async fn an_unannounced_answer_over_the_ceiling_is_refused() {
    let chunks = futures_util::stream::iter(
        (0..4).map(|_| Ok::<_, std::io::Error>(bytes::Bytes::from(vec![b'x'; 512]))),
    );
    let body = read(axum::body::Body::from_stream(chunks), 1024).await;
    assert_eq!(body, Err(BodyError::TooLarge));
}

/// An answer that says up front it is too large is refused unread.
#[tokio::test]
async fn an_announced_giant_is_refused() {
    let body = read(axum::body::Body::from(vec![b'x'; 4096]), 1024).await;
    assert_eq!(body, Err(BodyError::TooLarge));
}
