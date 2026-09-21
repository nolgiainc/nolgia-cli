//! `cargo add nolgia-client` must be enough to fetch the file a generation
//! produced (NOL-1087). These pin the two things that are easy to get wrong:
//! where the bearer token goes, and what is left on disk when a download
//! fails.

use nolgia_client::{ClientBuilder, ClientExt};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot really a png, but it is bytes";

fn carries_no_authorization(request: &Request) -> bool {
    !request.headers.contains_key("authorization")
}

fn carries_no_idempotency_key(request: &Request) -> bool {
    !request.headers.contains_key("idempotency-key")
}

/// An `asset.signed_url` points at a storage host and carries its own
/// credential in the query string. Our bearer token must not go with it: it
/// can spend money, and Google Cloud Storage refuses a V4 signed request that
/// also presents an Authorization header.
#[tokio::test]
async fn a_signed_url_on_another_origin_is_fetched_without_our_token() {
    let api = MockServer::start().await;
    let storage = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/bucket/first.png"))
        .and(carries_no_authorization)
        .and(carries_no_idempotency_key)
        .respond_with(ResponseTemplate::new(200).set_body_bytes(PNG))
        .expect(1)
        .mount(&storage)
        .await;

    let client = ClientBuilder::new(api.uri())
        .bearer_token("nol_test_token")
        .surface("test")
        .idempotency_key("key-1")
        .build()
        .expect("client builds");

    let directory = tempdir();
    let destination = directory.join("first.png");
    let signed = format!(
        "{}/bucket/first.png?X-Goog-Signature=deadbeefcafe",
        storage.uri()
    );

    let written = client
        .download(&signed, &destination)
        .await
        .expect("the download succeeds");

    assert_eq!(written, PNG.len() as u64);
    assert_eq!(std::fs::read(&destination).unwrap(), PNG);
    assert!(
        !partial(&destination).exists(),
        "the .part file must be renamed away, not left behind"
    );
    storage.verify().await;
}

/// The same call against our own API is authenticated, so a caller can pass a
/// `/v1/assets/{id}/content`-style URL and have it work.
#[tokio::test]
async fn a_url_on_our_own_api_keeps_the_clients_authorization() {
    let api = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/v1/assets/1/content"))
        .and(|request: &Request| {
            request
                .headers
                .get("authorization")
                .map(|value| value.as_bytes())
                == Some(b"Bearer nol_test_token".as_slice())
        })
        .respond_with(ResponseTemplate::new(200).set_body_bytes(PNG))
        .expect(1)
        .mount(&api)
        .await;

    let client = ClientBuilder::new(api.uri())
        .bearer_token("nol_test_token")
        .build()
        .expect("client builds");

    let body = client
        .download_bytes(&format!("{}/v1/assets/1/content", api.uri()))
        .await
        .expect("the download succeeds");

    assert_eq!(body, PNG);
    api.verify().await;
}

/// A signed URL expires. The message has to say so, and it must not print the
/// signature back at the user or into a log.
#[tokio::test]
async fn an_expired_url_explains_itself_and_leaves_nothing_on_disk() {
    let storage = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/bucket/gone.png"))
        .respond_with(ResponseTemplate::new(403).set_body_string("<Error>expired</Error>"))
        .mount(&storage)
        .await;

    let client = ClientBuilder::new("https://api.nolgia.ai")
        .bearer_token("nol_test_token")
        .build()
        .expect("client builds");

    let directory = tempdir();
    let destination = directory.join("gone.png");
    let signed = format!(
        "{}/bucket/gone.png?X-Goog-Signature=deadbeefcafe",
        storage.uri()
    );

    let error = client
        .download(&signed, &destination)
        .await
        .expect_err("403 is an error");

    let message = error.to_string();
    assert!(message.contains("403"), "{message}");
    assert!(message.contains("expires"), "{message}");
    assert!(!message.contains("deadbeefcafe"), "{message}");
    // Box<dyn Error> from `main` prints Debug, so it must be redacted too.
    assert!(!format!("{error:?}").contains("deadbeefcafe"));

    assert!(!destination.exists(), "a failed download wrote a file");
    assert!(
        !partial(&destination).exists(),
        "a .part file was left behind"
    );
}

#[tokio::test]
async fn a_string_that_is_not_a_url_fails_before_any_request() {
    let client = ClientBuilder::new("https://api.nolgia.ai")
        .bearer_token("nol_test_token")
        .build()
        .expect("client builds");

    let error = client
        .download_bytes("first.png")
        .await
        .expect_err("a bare filename is not a URL");
    assert!(error.to_string().contains("not a URL"), "{error}");
}

/// Downloading over an existing file replaces it rather than appending or
/// half-overwriting it.
#[tokio::test]
async fn a_second_download_replaces_the_first_file() {
    let storage = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/bucket/first.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(PNG))
        .mount(&storage)
        .await;

    let client = ClientBuilder::new("https://api.nolgia.ai")
        .bearer_token("nol_test_token")
        .build()
        .expect("client builds");

    let directory = tempdir();
    let destination = directory.join("first.png");
    std::fs::write(&destination, b"a much longer file that was already here").unwrap();

    client
        .download(&format!("{}/bucket/first.png", storage.uri()), &destination)
        .await
        .expect("the download succeeds");

    assert_eq!(std::fs::read(&destination).unwrap(), PNG);
}

fn partial(path: &std::path::Path) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".part");
    std::path::PathBuf::from(name)
}

/// A fresh directory under the OS temp dir. Deliberately not a dev-dependency:
/// the crate's dependency list is what customers install, and a test helper
/// this small is not worth another crate in the tree.
fn tempdir() -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "nolgia-client-download-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create the test directory");
    directory
}
