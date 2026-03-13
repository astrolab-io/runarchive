#[cfg(test)]
mod tests {
    use runarchive::parser;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[test]
    #[wasm_bindgen_test]
    fn test_find_eocd_signature_no_comment() {
        let zip_data: &[u8] = include_bytes!("fixtures/simple_archive_no_comment.zip");
        let eocd_offset = parser::find_eocd_offset(zip_data).expect("Failed to locate EOCD signature");
        assert_eq!(eocd_offset, zip_data.len() - 22);
    }

    #[test]
    #[wasm_bindgen_test]
    fn test_find_eocd_signature_with_large_comment() {
        let zip_data: &[u8] = include_bytes!("fixtures/archive_with_65535b_comment.zip");
        let eocd_offset = parser::find_eocd_offset(zip_data).unwrap();
        assert!(eocd_offset < zip_data.len() - 22);
    }
    
    #[test]
    #[wasm_bindgen_test]
    fn test_parse_eocd_and_central_directory() {
        let zip_data: &[u8] = include_bytes!("fixtures/simple_archive_no_comment.zip");
        
        let eocd_offset = parser::find_eocd_offset(zip_data).unwrap();
        let eocd = parser::parse_eocd(&zip_data[eocd_offset..]).unwrap();
        
        assert_eq!(eocd.total_cd_records, 1);
        
        // Parse the CDH based on the cd_offset
        let cd_offset = eocd.cd_offset as usize;
        let (cdh, bytes_read) = parser::parse_central_directory_header(&zip_data[cd_offset..]).unwrap();
        
        // Assert the filename is correct
        assert_eq!(cdh.file_name, "test.txt");
        assert!(bytes_read > 46);
    }
}
