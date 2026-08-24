//! Recoverable timestamped raw UDP recording format.

use std::{
    fs::{File, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use thiserror::Error;

const FILE_MAGIC: &[u8; 8] = b"OSCTKREC";
const CHUNK_MAGIC: &[u8; 4] = b"DGRM";
const VERSION: u16 = 1;
const HEADER_SIZE: u16 = 32;
const CHUNK_FIXED_BODY: usize = 16;
const MAX_BODY_SIZE: usize = 16 * 1024 * 1024;

/// One timestamped UDP datagram read from a recording.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Record {
    /// Nanoseconds since recording start, sampled from a monotonic clock.
    pub offset_ns: u64,
    /// Stable input endpoint identifier.
    pub stream_id: String,
    /// Byte-identical UDP payload.
    pub payload: Vec<u8>,
}

/// Metadata stored in the fixed file header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Metadata {
    /// Recording creation time as Unix epoch nanoseconds.
    pub start_unix_ns: u64,
}

/// A recording format or I/O failure.
#[derive(Debug, Error)]
pub enum RecordingError {
    /// Opening, reading, writing, flushing, or synchronizing failed.
    #[error("recording I/O failed for `{path}`: {source}")]
    Io {
        /// Recording path.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },
    /// The wall-clock value cannot be represented by the format.
    #[error("system clock is before Unix epoch or outside the recording timestamp range")]
    InvalidSystemTime,
    /// A stream identifier is too large.
    #[error("stream identifier is too large for recording format version 1")]
    StreamIdTooLarge,
    /// A payload or chunk exceeds format limits.
    #[error("datagram chunk is too large for recording format version 1")]
    ChunkTooLarge,
    /// Magic, version, reserved values, lengths, UTF-8, or checksum is invalid.
    #[error("invalid recording at byte {offset}: {reason}")]
    InvalidFormat {
        /// File offset where validation failed.
        offset: u64,
        /// Human-readable failure reason.
        reason: String,
    },
}

/// Sequential recording writer.
pub struct Writer {
    path: PathBuf,
    output: BufWriter<File>,
    origin: Instant,
    records: u64,
}

impl Writer {
    /// Creates a new recording and refuses to overwrite an existing file.
    ///
    /// # Errors
    ///
    /// Returns an error when the file exists, cannot be created, or the system
    /// clock cannot be represented.
    pub fn create(path: impl AsRef<Path>) -> Result<Self, RecordingError> {
        let path = path.as_ref().to_path_buf();
        let start_unix_ns: u64 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RecordingError::InvalidSystemTime)?
            .as_nanos()
            .try_into()
            .map_err(|_| RecordingError::InvalidSystemTime)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|source| io_error(&path, source))?;
        let mut output = BufWriter::new(file);
        let mut header = [0_u8; HEADER_SIZE as usize];
        header[..8].copy_from_slice(FILE_MAGIC);
        header[8..10].copy_from_slice(&VERSION.to_le_bytes());
        header[10..12].copy_from_slice(&HEADER_SIZE.to_le_bytes());
        header[16..24].copy_from_slice(&start_unix_ns.to_le_bytes());
        output
            .write_all(&header)
            .map_err(|source| io_error(&path, source))?;
        Ok(Self {
            path,
            output,
            origin: Instant::now(),
            records: 0,
        })
    }

    /// Writes one raw datagram with a timestamp sampled by the recorder.
    ///
    /// # Errors
    ///
    /// Returns an error when limits are exceeded or writing fails.
    pub fn write_datagram(
        &mut self,
        stream_id: &str,
        payload: &[u8],
    ) -> Result<(), RecordingError> {
        let offset_ns = self
            .origin
            .elapsed()
            .as_nanos()
            .try_into()
            .map_err(|_| RecordingError::ChunkTooLarge)?;
        self.write_record(offset_ns, stream_id, payload)
    }

    fn write_record(
        &mut self,
        offset_ns: u64,
        stream_id: &str,
        payload: &[u8],
    ) -> Result<(), RecordingError> {
        let stream_length: u16 = stream_id
            .len()
            .try_into()
            .map_err(|_| RecordingError::StreamIdTooLarge)?;
        let payload_length: u32 = payload
            .len()
            .try_into()
            .map_err(|_| RecordingError::ChunkTooLarge)?;
        let body_length = CHUNK_FIXED_BODY
            .checked_add(stream_id.len())
            .and_then(|length| length.checked_add(payload.len()))
            .filter(|length| *length <= MAX_BODY_SIZE)
            .ok_or(RecordingError::ChunkTooLarge)?;
        let body_length_u32: u32 = body_length
            .try_into()
            .map_err(|_| RecordingError::ChunkTooLarge)?;
        let mut body = Vec::with_capacity(body_length);
        body.extend_from_slice(&offset_ns.to_le_bytes());
        body.extend_from_slice(&stream_length.to_le_bytes());
        body.extend_from_slice(&0_u16.to_le_bytes());
        body.extend_from_slice(&payload_length.to_le_bytes());
        body.extend_from_slice(stream_id.as_bytes());
        body.extend_from_slice(payload);
        let checksum = crc32c::crc32c(&body);
        self.output
            .write_all(CHUNK_MAGIC)
            .and_then(|()| self.output.write_all(&body_length_u32.to_le_bytes()))
            .and_then(|()| self.output.write_all(&checksum.to_le_bytes()))
            .and_then(|()| self.output.write_all(&body))
            .map_err(|source| io_error(&self.path, source))?;
        self.records += 1;
        Ok(())
    }

    /// Flushes and synchronizes a clean recording.
    ///
    /// # Errors
    ///
    /// Returns an error when buffered data cannot be persisted.
    pub fn finish(mut self) -> Result<u64, RecordingError> {
        self.output
            .flush()
            .map_err(|source| io_error(&self.path, source))?;
        self.output
            .get_ref()
            .sync_all()
            .map_err(|source| io_error(&self.path, source))?;
        Ok(self.records)
    }
}

/// Reads and validates an entire recording.
///
/// # Errors
///
/// Returns an error for I/O failures, unknown versions, malformed records, or
/// damaged tails.
pub fn read_all(path: impl AsRef<Path>) -> Result<(Metadata, Vec<Record>), RecordingError> {
    let path = path.as_ref();
    let file = File::open(path).map_err(|source| io_error(path, source))?;
    let mut reader = BufReader::new(file);
    let mut header = [0_u8; HEADER_SIZE as usize];
    reader
        .read_exact(&mut header)
        .map_err(|source| io_error(path, source))?;
    if &header[..8] != FILE_MAGIC {
        return invalid(0, "incorrect file magic");
    }
    if u16::from_le_bytes([header[8], header[9]]) != VERSION {
        return invalid(8, "unsupported format version");
    }
    if u16::from_le_bytes([header[10], header[11]]) != HEADER_SIZE {
        return invalid(10, "incorrect header size");
    }
    if header[12..16]
        .iter()
        .chain(&header[24..32])
        .any(|byte| *byte != 0)
    {
        return invalid(12, "reserved header bytes are non-zero");
    }
    let mut start_bytes = [0_u8; 8];
    start_bytes.copy_from_slice(&header[16..24]);
    let start_unix_ns = u64::from_le_bytes(start_bytes);
    let mut records = Vec::new();
    let mut offset = u64::from(HEADER_SIZE);
    loop {
        let mut prefix = [0_u8; 12];
        match read_prefix(&mut reader, &mut prefix) {
            Ok(false) => break,
            Ok(true) => {}
            Err(source) => return invalid(offset, &format!("incomplete chunk prefix: {source}")),
        }
        if &prefix[..4] != CHUNK_MAGIC {
            return invalid(offset, "incorrect chunk magic");
        }
        let mut length_bytes = [0_u8; 4];
        length_bytes.copy_from_slice(&prefix[4..8]);
        let body_length = u32::from_le_bytes(length_bytes) as usize;
        if !(CHUNK_FIXED_BODY..=MAX_BODY_SIZE).contains(&body_length) {
            return invalid(offset, "invalid chunk body length");
        }
        let mut crc_bytes = [0_u8; 4];
        crc_bytes.copy_from_slice(&prefix[8..12]);
        let expected_crc = u32::from_le_bytes(crc_bytes);
        let mut body = vec![0_u8; body_length];
        reader
            .read_exact(&mut body)
            .map_err(|source| RecordingError::InvalidFormat {
                offset,
                reason: format!("incomplete chunk body: {source}"),
            })?;
        if crc32c::crc32c(&body) != expected_crc {
            return invalid(offset, "chunk CRC32C mismatch");
        }
        records.push(decode_body(offset, &body)?);
        offset = offset.saturating_add(12 + body_length as u64);
    }
    Ok((Metadata { start_unix_ns }, records))
}

fn decode_body(offset: u64, body: &[u8]) -> Result<Record, RecordingError> {
    let offset_ns = u64::from_le_bytes(body[..8].try_into().expect("fixed slice"));
    let stream_length = u16::from_le_bytes(body[8..10].try_into().expect("fixed slice")) as usize;
    let flags = u16::from_le_bytes(body[10..12].try_into().expect("fixed slice"));
    let payload_length = u32::from_le_bytes(body[12..16].try_into().expect("fixed slice")) as usize;
    if flags != 0 {
        return invalid(offset, "unsupported chunk flags");
    }
    if CHUNK_FIXED_BODY + stream_length + payload_length != body.len() {
        return invalid(offset, "chunk component lengths do not match body length");
    }
    let stream_end = CHUNK_FIXED_BODY + stream_length;
    let stream_id = std::str::from_utf8(&body[CHUNK_FIXED_BODY..stream_end])
        .map_err(|_| RecordingError::InvalidFormat {
            offset,
            reason: "stream identifier is not UTF-8".to_owned(),
        })?
        .to_owned();
    Ok(Record {
        offset_ns,
        stream_id,
        payload: body[stream_end..].to_vec(),
    })
}

fn read_prefix(reader: &mut impl Read, prefix: &mut [u8; 12]) -> io::Result<bool> {
    let read = reader.read(&mut prefix[..1])?;
    if read == 0 {
        return Ok(false);
    }
    reader.read_exact(&mut prefix[1..])?;
    Ok(true)
}

fn io_error(path: &Path, source: io::Error) -> RecordingError {
    RecordingError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn invalid<T>(offset: u64, reason: &str) -> Result<T, RecordingError> {
    Err(RecordingError::InvalidFormat {
        offset,
        reason: reason.to_owned(),
    })
}

/// Returns the replay delay between two monotonic offsets.
#[must_use]
pub fn replay_delay(previous_ns: u64, current_ns: u64) -> Duration {
    Duration::from_nanos(current_ns.saturating_sub(previous_ns))
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use super::{RecordingError, Writer, read_all, replay_delay};

    fn path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("osc-recording-{}-{name}", std::process::id()))
    }

    #[test]
    fn round_trips_payload_stream_and_timestamps() {
        let path = path("roundtrip.oscr");
        let _ = fs::remove_file(&path);
        let mut writer = Writer::create(&path).expect("writer should be created");
        writer
            .write_record(10, "face", b"first")
            .expect("first record should write");
        writer
            .write_record(25, "body", b"second")
            .expect("second record should write");
        assert_eq!(writer.finish().expect("writer should finish"), 2);
        let (_, records) = read_all(&path).expect("recording should read");
        assert_eq!(records[0].offset_ns, 10);
        assert_eq!(records[0].stream_id, "face");
        assert_eq!(records[0].payload, b"first");
        assert_eq!(records[1].offset_ns, 25);
        assert_eq!(replay_delay(10, 25).as_nanos(), 15);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn refuses_to_overwrite_existing_recording() {
        let path = path("exists.oscr");
        let _ = fs::remove_file(&path);
        let writer = Writer::create(&path).expect("first writer should be created");
        assert!(matches!(
            Writer::create(&path),
            Err(RecordingError::Io { .. })
        ));
        writer.finish().expect("writer should finish");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn detects_a_truncated_tail() {
        let path = path("truncated.oscr");
        let _ = fs::remove_file(&path);
        let mut writer = Writer::create(&path).expect("writer should be created");
        writer
            .write_record(1, "face", b"payload")
            .expect("record should write");
        writer.finish().expect("writer should finish");
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("recording should reopen")
            .write_all(b"DGRM")
            .expect("partial tail should write");
        assert!(matches!(
            read_all(&path),
            Err(RecordingError::InvalidFormat { .. })
        ));
        let _ = fs::remove_file(path);
    }
}
