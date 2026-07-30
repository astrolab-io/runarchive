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

    /// A central-directory header with `0xFFFFFFFF` in a size field is not a
    /// 4 GiB entry — it is a zip64 entry whose real size lives in the extra
    /// field. Taking the sentinel literally turns a 1 GiB member into a 4 GiB
    /// range request that no origin can satisfy.
    #[test]
    #[wasm_bindgen_test]
    fn test_zip64_extent_resolves_sentinel_sizes() {
        // Header of IBGE's 35_SP.zip: one deflated member, both sizes
        // sentinelled, offset small enough to stay 32-bit.
        let cdh = central_directory_header(
            0xFFFF_FFFF,
            0xFFFF_FFFF,
            0,
            &zip64_extra(&[3_829_859_937, 1_082_015_276]),
        );

        let (cdh, _) = parser::parse_central_directory_header(&cdh).unwrap();
        let extent = cdh.extent();

        assert_eq!(extent.uncompressed_size, 3_829_859_937);
        assert_eq!(extent.compressed_size, 1_082_015_276);
        assert_eq!(extent.local_header_offset, 0);
    }

    /// Only the overflowed fields are present in the extra field, in spec
    /// order, so a lone sentinel must consume the first `u64` and the fields
    /// that fit must keep their header values.
    #[test]
    #[wasm_bindgen_test]
    fn test_zip64_extent_resolves_only_sentinelled_fields() {
        let cdh = central_directory_header(1024, 4096, 0xFFFF_FFFF, &zip64_extra(&[8_000_000_000]));

        let (cdh, _) = parser::parse_central_directory_header(&cdh).unwrap();
        let extent = cdh.extent();

        assert_eq!(extent.uncompressed_size, 4096);
        assert_eq!(extent.compressed_size, 1024);
        assert_eq!(extent.local_header_offset, 8_000_000_000);
    }

    /// A plain zip32 entry has no extra field at all — the header values are
    /// the answer.
    #[test]
    #[wasm_bindgen_test]
    fn test_extent_without_zip64_extra_field() {
        let cdh = central_directory_header(1024, 4096, 512, &[]);

        let (cdh, _) = parser::parse_central_directory_header(&cdh).unwrap();
        let extent = cdh.extent();

        assert_eq!(extent.uncompressed_size, 4096);
        assert_eq!(extent.compressed_size, 1024);
        assert_eq!(extent.local_header_offset, 512);
    }

    /// A zip64 extended information block: header id `0x0001`, byte length,
    /// then one `u64` per overflowed field.
    fn zip64_extra(values: &[u64]) -> Vec<u8> {
        let mut out = vec![0x01, 0x00];
        out.extend_from_slice(&((values.len() * 8) as u16).to_le_bytes());
        for v in values {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// A minimal central-directory header for `test.txt`, deflated, with the
    /// given sizes, offset and extra field.
    fn central_directory_header(
        compressed_size: u32,
        uncompressed_size: u32,
        local_header_offset: u32,
        extra_field: &[u8],
    ) -> Vec<u8> {
        const NAME: &[u8] = b"test.txt";

        let mut out = Vec::new();
        out.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]); // signature
        out.extend_from_slice(&45u16.to_le_bytes()); // version made by
        out.extend_from_slice(&45u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&8u16.to_le_bytes()); // deflate
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&0u32.to_le_bytes()); // crc32
        out.extend_from_slice(&compressed_size.to_le_bytes());
        out.extend_from_slice(&uncompressed_size.to_le_bytes());
        out.extend_from_slice(&(NAME.len() as u16).to_le_bytes());
        out.extend_from_slice(&(extra_field.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment length
        out.extend_from_slice(&0u16.to_le_bytes()); // disk number start
        out.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        out.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        out.extend_from_slice(&local_header_offset.to_le_bytes());
        out.extend_from_slice(NAME);
        out.extend_from_slice(extra_field);
        out
    }
}
