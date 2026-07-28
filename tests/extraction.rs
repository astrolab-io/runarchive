#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
#[cfg(any(feature = "async-futures", feature = "async-tokio"))]
mod tests {
    use runarchive::Archive;
    use std::io::Write;

    #[cfg(feature = "async-tokio")]
    use tokio::io::AsyncReadExt;
    #[cfg(feature = "async-futures")]
    use futures::io::AsyncReadExt;

    #[tokio::test]
    async fn test_extract_stored_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let zip_path = temp_dir.path().join("test.zip");
        
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
            zip.start_file("test.txt", options).unwrap();
            zip.write_all(b"Hello World").unwrap();
            zip.finish().unwrap();
        }

        let uri = format!("file://{}/test.zip", temp_dir.path().display());
        let mut archive = Archive::open(&uri).await.unwrap();
        
        let files = archive.list_files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name.as_ref(), "test.txt");
        assert_eq!(files[0].compression_method, 0);

        let mut reader = archive.extract_file("test.txt").await.unwrap();
        let mut output = Vec::new();
        reader.read_to_end(&mut output).await.unwrap();
        assert_eq!(output, b"Hello World");
    }

    #[tokio::test]
    async fn test_extract_deflate_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let zip_path = temp_dir.path().join("test_deflate.zip");
        
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("test.txt", options).unwrap();
            zip.write_all(b"Hello World from deflate compressed file").unwrap();
            zip.finish().unwrap();
        }

        let uri = format!("file://{}/test_deflate.zip", temp_dir.path().display());
        let mut archive = Archive::open(&uri).await.unwrap();
        
        let files = archive.list_files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name.as_ref(), "test.txt");
        assert_eq!(files[0].compression_method, 8);

        let mut reader = archive.extract_file("test.txt").await.unwrap();
        let mut output = Vec::new();
        reader.read_to_end(&mut output).await.unwrap();
        assert_eq!(output, b"Hello World from deflate compressed file");
    }

    #[tokio::test]
    async fn test_extract_multiple_files() {
        let temp_dir = tempfile::tempdir().unwrap();
        let zip_path = temp_dir.path().join("multi.zip");
        
        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            
            zip.start_file("file1.txt", zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
            zip.write_all(b"Content of file 1").unwrap();
            
            zip.start_file("file2.txt", zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
            zip.write_all(b"Content of file 2 is longer").unwrap();
            
            zip.finish().unwrap();
        }

        let uri = format!("file://{}/multi.zip", temp_dir.path().display());
        let mut archive = Archive::open(&uri).await.unwrap();
        
        let files = archive.list_files();
        assert_eq!(files.len(), 2);

        let mut reader1 = archive.extract_file("file1.txt").await.unwrap();
        let mut output1 = Vec::new();
        reader1.read_to_end(&mut output1).await.unwrap();
        assert_eq!(output1, b"Content of file 1");

        let mut reader2 = archive.extract_file("file2.txt").await.unwrap();
        let mut output2 = Vec::new();
        reader2.read_to_end(&mut output2).await.unwrap();
        assert_eq!(output2, b"Content of file 2 is longer");
    }
}