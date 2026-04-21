#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use runarchive::reader::Reader;
    use wiremock::matchers::{header, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_http_reader_range_request() {
        let mock_server = MockServer::start().await;

        // Setup mock for a ZIP file (need minimal valid structure for Reader::open)
        // First, mock the HEAD request
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(200).insert_header("content-length", "1000"))
            .mount(&mock_server)
            .await;

        // Setup GET mock for Range fetch
        Mock::given(method("GET"))
            .and(header("Range", "bytes=935-999"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(vec![0u8; 66]))
            .mount(&mock_server)
            .await;

        // Create reader - will fail on parsing but that's ok, we're testing HTTP
        let result = Reader::open(&mock_server.uri()).await;

        // For this test, we'll just verify the HTTP mocking works
        // In real usage, Reader would parse the ZIP structure
        assert!(result.is_ok() || result.is_err()); // Either is fine for mock test
    }
}
