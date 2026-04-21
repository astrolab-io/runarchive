#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use dhat::HeapStats;
    use std::io::Write;

    #[global_allocator]
    static ALLOC: dhat::Alloc = dhat::Alloc;

    #[test]
    fn parser_find_eocd_offset_no_heap() {
        let _profiler = dhat::Profiler::builder().testing().build();
        let zip_data: &[u8] = include_bytes!("fixtures/simple_archive_no_comment.zip");

        let result = runarchive::parser::find_eocd_offset(zip_data);
        assert!(result.is_ok());

        let stats = HeapStats::get();
        dhat::assert_eq!(stats.total_bytes, 0);
        dhat::assert_eq!(stats.total_blocks, 0);
    }

    #[test]
    fn parser_parse_eocd_no_heap() {
        let _profiler = dhat::Profiler::builder().testing().build();
        let zip_data: &[u8] = include_bytes!("fixtures/simple_archive_no_comment.zip");

        let eocd_offset = runarchive::parser::find_eocd_offset(zip_data).unwrap();
        let result = runarchive::parser::parse_eocd(&zip_data[eocd_offset..]);
        assert!(result.is_ok());

        let stats = HeapStats::get();
        dhat::assert_eq!(stats.total_bytes, 0);
        dhat::assert_eq!(stats.total_blocks, 0);
    }

    #[test]
    fn parser_parse_central_directory_header_no_heap() {
        let _profiler = dhat::Profiler::builder().testing().build();
        let zip_data: &[u8] = include_bytes!("fixtures/simple_archive_no_comment.zip");

        let eocd_offset = runarchive::parser::find_eocd_offset(zip_data).unwrap();
        let eocd = runarchive::parser::parse_eocd(&zip_data[eocd_offset..]).unwrap();
        let cd_data = &zip_data[eocd.cd_offset as usize..];

        let (header, _bytes_read) =
            runarchive::parser::parse_central_directory_header(cd_data).unwrap();
        dhat::assert_eq!(header.file_name, "test.txt");

        let stats = HeapStats::get();
        dhat::assert_eq!(stats.total_bytes, 0);
        dhat::assert_eq!(stats.total_blocks, 0);
    }

    #[test]
    fn parser_find_zip64_locator_no_heap() {
        let _profiler = dhat::Profiler::builder().testing().build();
        let zip_data: &[u8] = include_bytes!("fixtures/simple_archive_no_comment.zip");

        let result = runarchive::parser::find_zip64_locator(zip_data);
        assert!(result.is_err());

        let stats = HeapStats::get();
        dhat::assert_eq!(stats.total_bytes, 0);
        dhat::assert_eq!(stats.total_blocks, 0);
    }

    #[test]
    fn eocd_parse_all_fields_zero_copy() {
        let zip_data: &[u8] = include_bytes!("fixtures/simple_archive_no_comment.zip");

        let _profiler = dhat::Profiler::builder().testing().build();

        let eocd_offset = runarchive::parser::find_eocd_offset(zip_data).unwrap();
        let eocd = runarchive::parser::parse_eocd(&zip_data[eocd_offset..]).unwrap();

        dhat::assert_eq!(eocd.disk_number, 0);
        dhat::assert_eq!(eocd.disk_with_cd, 0);
        dhat::assert_eq!(eocd.cd_records_on_disk, 1);
        dhat::assert_eq!(eocd.total_cd_records, 1);
        dhat::assert_eq!(eocd.cd_size, 54);
        dhat::assert_eq!(eocd.cd_offset, 49);
        dhat::assert_eq!(eocd.comment_length, 0);

        let stats = HeapStats::get();
        dhat::assert_eq!(stats.total_bytes, 0);
        dhat::assert_eq!(stats.total_blocks, 0);
    }

    #[test]
    fn archive_open_no_heap_for_parsing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let zip_path = temp_dir.path().join("test.zip");

        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("test.txt", options).unwrap();
            zip.write_all(b"Content").unwrap();
            zip.finish().unwrap();
        }

        let uri = format!("file://{}/test.zip", temp_dir.path().display());

        let archive = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runarchive::Archive::open(&uri))
            .unwrap();

        let _profiler = dhat::Profiler::builder().testing().build();

        let files = archive.list_files();
        dhat::assert_eq!(files.len(), 1);

        let stats = HeapStats::get();
        dhat::assert_eq!(stats.total_bytes, 0);
        dhat::assert_eq!(stats.total_blocks, 0);
    }

    #[test]
    fn archive_file_entry_name_arc_no_heap_on_open() {
        let temp_dir = tempfile::tempdir().unwrap();
        let zip_path = temp_dir.path().join("test.zip");

        {
            let file = std::fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("tiny.txt", options).unwrap();
            zip.write_all(b"X").unwrap();
            zip.finish().unwrap();
        }

        let uri = format!("file://{}/test.zip", temp_dir.path().display());

        let archive = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(runarchive::Archive::open(&uri))
            .unwrap();

        let entry = &archive.list_files()[0];

        let _profiler = dhat::Profiler::builder().testing().build();

        let name = entry.name.as_ref();
        dhat::assert_eq!(name, "tiny.txt");

        let stats = HeapStats::get();
        dhat::assert_eq!(stats.total_bytes, 0);
        dhat::assert_eq!(stats.total_blocks, 0);
    }

    // TODO: Find a way to test internal allocation (without taking into account external dependencies allocations)

    // #[test]
    // fn reader_read_range_returns_bytes_zero_copy() {
    //     let temp_dir = tempfile::tempdir().unwrap();
    //     let zip_path = temp_dir.path().join("test.zip");

    //     {
    //         let file = std::fs::File::create(&zip_path).unwrap();
    //         let mut zip = zip::ZipWriter::new(file);
    //         let options = zip::write::FileOptions::default()
    //             .compression_method(zip::CompressionMethod::Stored);
    //         zip.start_file("test.txt", options).unwrap();
    //         zip.write_all(b"Hello World").unwrap();
    //         zip.finish().unwrap();
    //     }

    //     let uri = format!("file://{}/test.zip", temp_dir.path().display());
    //     let runtime = tokio::runtime::Runtime::new().unwrap();
    //     let reader = runtime
    //         .block_on(runarchive::reader::Reader::open(&uri))
    //         .unwrap();

    //     let _profiler = dhat::Profiler::builder().testing().build();

    //     let file_size = reader.file_size();
    //     let result = runtime
    //         .block_on(reader.read_range(file_size - 22, 22))
    //         .unwrap();
    //     dhat::assert_eq!(result.len(), 22);

    //     let stats = HeapStats::get();
    //     dhat::assert_eq!(stats.total_bytes, 0);
    //     dhat::assert_eq!(stats.total_blocks, 0);
    // }

    // #[tokio::test]
    // async fn extract_file_no_heap_allocation() {
    //     let temp_dir = tempfile::tempdir().unwrap();
    //     let zip_path = temp_dir.path().join("test.zip");

    //     {
    //         let file = std::fs::File::create(&zip_path).unwrap();
    //         let mut zip = zip::ZipWriter::new(file);
    //         let options = zip::write::FileOptions::default()
    //             .compression_method(zip::CompressionMethod::Stored);
    //         zip.start_file("test.txt", options).unwrap();
    //         zip.write_all(b"Hello").unwrap();
    //         zip.finish().unwrap();
    //     }

    //     let uri = format!("file://{}/test.zip", temp_dir.path().display());
    //     let mut archive = runarchive::Archive::open(&uri).await.unwrap();
    //     let _profiler = dhat::Profiler::builder().testing().build();

    //     let mut reader = archive.extract_file("test.txt").await.unwrap();
    //     let mut buf = Vec::new();
    //     use tokio::io::AsyncReadExt;
    //     reader.read_to_end(&mut buf).await.unwrap();
    //     dhat::assert_eq!(&buf, b"Hello");

    //     let stats = HeapStats::get();
    //     dhat::assert_eq!(stats.total_bytes, 0);
    //     dhat::assert_eq!(stats.total_blocks, 0);
    // }

    // #[tokio::test]
    // async fn reader_stream_returns_boxed_stream_zero_copy() {
    //     let temp_dir = tempfile::tempdir().unwrap();
    //     let zip_path = temp_dir.path().join("test.zip");

    //     {
    //         let file = std::fs::File::create(&zip_path).unwrap();
    //         let mut zip = zip::ZipWriter::new(file);
    //         let options = zip::write::FileOptions::default()
    //             .compression_method(zip::CompressionMethod::Stored);
    //         zip.start_file("test.txt", options).unwrap();
    //         zip.write_all(b"Stream test content").unwrap();
    //         zip.finish().unwrap();
    //     }

    //     let uri = format!("file://{}/test.zip", temp_dir.path().display());
    //     let reader = runarchive::reader::Reader::open(&uri).await.unwrap();
    //     let file_size = reader.file_size();

    //     let _profiler = dhat::Profiler::builder().testing().build();

    //     let stream = reader.stream(file_size - 22..file_size).await.unwrap();
    //     use futures::StreamExt;
    //     let chunks: Vec<Result<bytes::Bytes, _>> = stream.take(5).collect().await;
    //     dhat::assert!(!chunks.is_empty());

    //     let stats = HeapStats::get();
    //     dhat::assert_eq!(stats.total_bytes, 0);
    //     dhat::assert_eq!(stats.total_blocks, 0);
    // }
}
