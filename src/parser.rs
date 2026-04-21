#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    SignatureNotFound,
    InvalidFormat,
    Utf8Error,
}

const EOCD_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
const CDH_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
const ZIP64_EOCD_LOCATOR_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x06, 0x07];
const ZIP64_EOCD_RECORD_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x06, 0x06];

const MIN_EOCD_SIZE: usize = 22;
const MAX_EOCD_SIZE: usize = 65557;

#[derive(Debug, PartialEq, Eq)]
pub struct EocdRecord {
    pub disk_number: u16,
    pub disk_with_cd: u16,
    pub cd_records_on_disk: u16,
    pub total_cd_records: u16,
    pub cd_size: u32,
    pub cd_offset: u32,
    pub comment_length: u16,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Zip64Locator {
    pub disk_with_zip64_eocd: u32,
    pub zip64_eocd_offset: u64,
    pub total_disks: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Zip64EocdRecord {
    pub total_cd_records: u64,
    pub cd_size: u64,
    pub cd_offset: u64,
}

pub fn find_eocd_offset(data: &[u8]) -> Result<usize, ParseError> {
    let len = data.len();
    if len < MIN_EOCD_SIZE {
        return Err(ParseError::InvalidFormat);
    }
    let scan_limit = std::cmp::max(len.saturating_sub(MAX_EOCD_SIZE), 0);
    let start_pos = len - MIN_EOCD_SIZE;

    for i in (scan_limit..=start_pos).rev() {
        if data[i..i + 4] == EOCD_SIGNATURE {
            return Ok(i);
        }
    }

    Err(ParseError::SignatureNotFound)
}

pub fn parse_eocd(data: &[u8]) -> Result<EocdRecord, ParseError> {
    if data.len() < MIN_EOCD_SIZE {
        return Err(ParseError::InvalidFormat);
    }

    Ok(EocdRecord {
        disk_number: u16::from_le_bytes([data[4], data[5]]),
        disk_with_cd: u16::from_le_bytes([data[6], data[7]]),
        cd_records_on_disk: u16::from_le_bytes([data[8], data[9]]),
        total_cd_records: u16::from_le_bytes([data[10], data[11]]),
        cd_size: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
        cd_offset: u32::from_le_bytes([data[16], data[17], data[18], data[19]]),
        comment_length: u16::from_le_bytes([data[20], data[21]]),
    })
}

pub fn parse_zip64_locator(data: &[u8]) -> Result<Zip64Locator, ParseError> {
    if data.len() < 20 {
        return Err(ParseError::InvalidFormat);
    }
    if data[0..4] != ZIP64_EOCD_LOCATOR_SIGNATURE {
        return Err(ParseError::SignatureNotFound);
    }

    Ok(Zip64Locator {
        disk_with_zip64_eocd: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
        zip64_eocd_offset: u64::from_le_bytes([
            data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
        ]),
        total_disks: u32::from_le_bytes([data[16], data[17], data[18], data[19]]),
    })
}

pub fn parse_zip64_eocd(data: &[u8]) -> Result<Zip64EocdRecord, ParseError> {
    if data.len() < 56 {
        return Err(ParseError::InvalidFormat);
    }
    if data[0..4] != ZIP64_EOCD_RECORD_SIGNATURE {
        return Err(ParseError::SignatureNotFound);
    }

    Ok(Zip64EocdRecord {
        total_cd_records: u64::from_le_bytes([
            data[32], data[33], data[34], data[35], data[36], data[37], data[38], data[39],
        ]),
        cd_size: u64::from_le_bytes([
            data[40], data[41], data[42], data[43], data[44], data[45], data[46], data[47],
        ]),
        cd_offset: u64::from_le_bytes([
            data[48], data[49], data[50], data[51], data[52], data[53], data[54], data[55],
        ]),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct CentralDirectoryHeader<'a> {
    pub version_made_by: u16,
    pub version_needed: u16,
    pub flags: u16,
    pub compression_method: u16,
    pub last_mod_time: u16,
    pub last_mod_date: u16,
    pub crc32: u32,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub file_name: &'a str,
    pub extra_field: &'a [u8],
    pub file_comment: &'a [u8],
    pub local_header_offset: u32,
}

pub fn parse_central_directory_header<'a>(
    data: &'a [u8],
) -> Result<(CentralDirectoryHeader<'a>, usize), ParseError> {
    if data.len() < 46 {
        return Err(ParseError::InvalidFormat);
    }

    if data[0..4] != CDH_SIGNATURE {
        return Err(ParseError::SignatureNotFound);
    }

    let file_name_len = u16::from_le_bytes([data[28], data[29]]) as usize;
    let extra_field_len = u16::from_le_bytes([data[30], data[31]]) as usize;
    let file_comment_len = u16::from_le_bytes([data[32], data[33]]) as usize;

    let total_len = 46 + file_name_len + extra_field_len + file_comment_len;
    if data.len() < total_len {
        return Err(ParseError::InvalidFormat);
    }

    let file_name_bytes = &data[46..46 + file_name_len];
    let file_name = std::str::from_utf8(file_name_bytes).map_err(|_| ParseError::Utf8Error)?;

    let extra_field = &data[46 + file_name_len..46 + file_name_len + extra_field_len];
    let file_comment = &data[46 + file_name_len + extra_field_len..total_len];

    let header = CentralDirectoryHeader {
        version_made_by: u16::from_le_bytes([data[4], data[5]]),
        version_needed: u16::from_le_bytes([data[6], data[7]]),
        flags: u16::from_le_bytes([data[8], data[9]]),
        compression_method: u16::from_le_bytes([data[10], data[11]]),
        last_mod_time: u16::from_le_bytes([data[12], data[13]]),
        last_mod_date: u16::from_le_bytes([data[14], data[15]]),
        crc32: u32::from_le_bytes([data[16], data[17], data[18], data[19]]),
        compressed_size: u32::from_le_bytes([data[20], data[21], data[22], data[23]]),
        uncompressed_size: u32::from_le_bytes([data[24], data[25], data[26], data[27]]),
        file_name,
        extra_field,
        file_comment,
        local_header_offset: u32::from_le_bytes([data[42], data[43], data[44], data[45]]),
    };

    Ok((header, total_len))
}

pub fn iterate_central_directory<'a>(
    mut cd_data: &'a [u8],
    total_records: u16,
) -> Result<Vec<CentralDirectoryHeader<'a>>, ParseError> {
    let mut headers = Vec::with_capacity(total_records as usize);
    for _ in 0..total_records {
        if cd_data.is_empty() {
            break; // Unexpected end of data
        }
        let (header, bytes_read) = parse_central_directory_header(cd_data)?;
        headers.push(header);
        cd_data = &cd_data[bytes_read..];
    }
    Ok(headers)
}

pub fn find_zip64_locator(data: &[u8]) -> Result<usize, ParseError> {
    let len = data.len();
    if len < 20 {
        return Err(ParseError::InvalidFormat);
    }
    let scan_limit = std::cmp::max(len.saturating_sub(MAX_EOCD_SIZE), 0);
    let start_pos = len - 20;

    for i in (scan_limit..=start_pos).rev() {
        if data[i..i + 4] == ZIP64_EOCD_LOCATOR_SIGNATURE {
            return Ok(i);
        }
    }

    Err(ParseError::SignatureNotFound)
}
