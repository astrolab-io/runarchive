#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use runarchive::seeker::{Seeker, http::HttpSeeker};
    use std::io::SeekFrom;
    use wiremock::matchers::{method, header};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_http_seeker_range_request() {
        let mock_server = MockServer::start().await;

        // Setup GET mock for standard Range fetch
        Mock::given(method("GET"))
            .and(header("Range", "bytes=100-199"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(vec![0u8; 100]))
            .mount(&mock_server)
            .await;

        let mut seeker = HttpSeeker::new_with_size(&mock_server.uri(), 1000).await.expect("Failed to create seeker");
        assert_eq!(seeker.file_size(), 1000);
        
        // Seek to 100 and read 100 bytes
        seeker.seek(SeekFrom::Start(100)).await.unwrap();
        
        let mut buf = vec![0u8; 100];
        let bytes_read = seeker.read(&mut buf).await.unwrap();
        assert_eq!(bytes_read, 100);
        
        // Check fallback behavior: No 206 returned
        Mock::given(method("GET"))
            .and(header("Range", "bytes=200-299"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 1000])) // Malicious/misconfigured server
            .mount(&mock_server)
            .await;
            
        seeker.seek(SeekFrom::Start(200)).await.unwrap();
        let fallback_read_res = seeker.read(&mut buf).await;
        assert!(fallback_read_res.is_err(), "Expected error when server doesn't respect range request");
    }
}
