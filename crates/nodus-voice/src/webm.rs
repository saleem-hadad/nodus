//! WebM as Chromium's MediaRecorder writes it for `audio/webm;codecs=opus`
//! (VOICE-021): its `WebmMuxer` drives libwebm's mkvmuxer in live mode, so
//! the Segment and every Cluster have an unknown size, there are no Cues or
//! SeekHead, and the file can't be seeked or tell its duration. The one Opus
//! track is laid out as mkvmuxer lays it out, with Chromium's OpusHead
//! (pre-skip 0). A new Cluster starts only when a block's time would no
//! longer fit its 16-bit offset.

/// EBML's "unknown size": all ones in eight bytes.
const UNKNOWN_SIZE: [u8; 8] = [0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
/// The largest block time offset in a Cluster (`kMaxBlockTimecode`).
const MAX_BLOCK_TIMECODE: u64 = 0x7FFF;

mod id {
    pub const EBML: u32 = 0x1A45_DFA3;
    pub const EBML_VERSION: u32 = 0x4286;
    pub const EBML_READ_VERSION: u32 = 0x42F7;
    pub const EBML_MAX_ID_LENGTH: u32 = 0x42F2;
    pub const EBML_MAX_SIZE_LENGTH: u32 = 0x42F3;
    pub const DOC_TYPE: u32 = 0x4282;
    pub const DOC_TYPE_VERSION: u32 = 0x4287;
    pub const DOC_TYPE_READ_VERSION: u32 = 0x4285;
    pub const SEGMENT: u32 = 0x1853_8067;
    pub const INFO: u32 = 0x1549_A966;
    pub const TIMECODE_SCALE: u32 = 0x2A_D7B1;
    pub const MUXING_APP: u32 = 0x4D80;
    pub const WRITING_APP: u32 = 0x5741;
    pub const TRACKS: u32 = 0x1654_AE6B;
    pub const TRACK_ENTRY: u32 = 0xAE;
    pub const TRACK_NUMBER: u32 = 0xD7;
    pub const TRACK_UID: u32 = 0x73C5;
    pub const TRACK_TYPE: u32 = 0x83;
    pub const CODEC_ID: u32 = 0x86;
    pub const CODEC_PRIVATE: u32 = 0x63A2;
    pub const AUDIO: u32 = 0xE1;
    pub const SAMPLING_FREQUENCY: u32 = 0xB5;
    pub const CHANNELS: u32 = 0x9F;
    pub const BIT_DEPTH: u32 = 0x6264;
    pub const CLUSTER: u32 = 0x1F43_B675;
    pub const TIMECODE: u32 = 0xE7;
    pub const SIMPLE_BLOCK: u32 = 0xA3;
}

fn put_id(out: &mut Vec<u8>, id: u32) {
    let bytes = id.to_be_bytes();
    let skip = bytes.iter().take_while(|&&b| b == 0).count();
    out.extend_from_slice(&bytes[skip..]);
}

/// An EBML size in the fewest bytes (`GetCodedUIntSize`).
fn put_size(out: &mut Vec<u8>, size: u64) {
    let mut len = 1;
    while len < 8 && size >= (1u64 << (7 * len)) - 1 {
        len += 1;
    }
    let marked = size | (1u64 << (7 * len));
    out.extend_from_slice(&marked.to_be_bytes()[8 - len..]);
}

fn element(out: &mut Vec<u8>, id: u32, body: &[u8]) {
    put_id(out, id);
    put_size(out, body.len() as u64);
    out.extend_from_slice(body);
}

/// An unsigned integer in the fewest bytes (at least one).
fn uint(out: &mut Vec<u8>, id: u32, value: u64) {
    let bytes = value.to_be_bytes();
    let skip = bytes.iter().take_while(|&&b| b == 0).count().min(7);
    element(out, id, &bytes[skip..]);
}

/// Chromium's OpusHead (`WriteOpusHeader`): version 1, the channel count, a
/// pre-skip of 0, the sample rate, no gain, mapping family 0.
pub fn opus_head(channels: u8, sample_rate: u32) -> [u8; 19] {
    let mut head = [0u8; 19];
    head[..8].copy_from_slice(b"OpusHead");
    head[8] = 1;
    head[9] = channels;
    head[12..16].copy_from_slice(&sample_rate.to_le_bytes());
    head
}

/// Writes a WebM file of one Opus track, block by block.
#[derive(Debug, Clone)]
pub struct WebmWriter {
    out: Vec<u8>,
    cluster: Option<u64>,
}

impl WebmWriter {
    /// The headers, for an Opus track of `channels` at `sample_rate`, with
    /// `track_uid` (mkvmuxer makes one from seven random bytes).
    pub fn new(channels: u8, sample_rate: u32, track_uid: u64) -> Self {
        let mut out = Vec::new();
        let mut ebml = Vec::new();
        uint(&mut ebml, id::EBML_VERSION, 1);
        uint(&mut ebml, id::EBML_READ_VERSION, 1);
        uint(&mut ebml, id::EBML_MAX_ID_LENGTH, 4);
        uint(&mut ebml, id::EBML_MAX_SIZE_LENGTH, 8);
        element(&mut ebml, id::DOC_TYPE, b"webm");
        uint(&mut ebml, id::DOC_TYPE_VERSION, 4);
        uint(&mut ebml, id::DOC_TYPE_READ_VERSION, 2);
        element(&mut out, id::EBML, &ebml);

        put_id(&mut out, id::SEGMENT);
        out.extend_from_slice(&UNKNOWN_SIZE);

        let mut info = Vec::new();
        uint(&mut info, id::TIMECODE_SCALE, 1_000_000);
        element(&mut info, id::MUXING_APP, b"Chrome");
        element(&mut info, id::WRITING_APP, b"Chrome");
        element(&mut out, id::INFO, &info);

        let mut audio = Vec::new();
        element(&mut audio, id::SAMPLING_FREQUENCY, &(sample_rate as f32).to_be_bytes());
        uint(&mut audio, id::CHANNELS, u64::from(channels));
        // "Audio data is always pcm_f32le."
        uint(&mut audio, id::BIT_DEPTH, 32);
        let mut entry = Vec::new();
        uint(&mut entry, id::TRACK_NUMBER, 1);
        uint(&mut entry, id::TRACK_UID, track_uid);
        uint(&mut entry, id::TRACK_TYPE, 2);
        element(&mut entry, id::CODEC_ID, b"A_OPUS");
        element(&mut entry, id::CODEC_PRIVATE, &opus_head(channels, sample_rate));
        element(&mut entry, id::AUDIO, &audio);
        let mut tracks = Vec::new();
        element(&mut tracks, id::TRACK_ENTRY, &entry);
        element(&mut out, id::TRACKS, &tracks);
        Self { out, cluster: None }
    }

    /// Adds an Opus packet that starts `time_ms` into the recording, as a
    /// key-frame SimpleBlock on track 1.
    pub fn add_packet(&mut self, time_ms: u64, packet: &[u8]) {
        let cluster = match self.cluster {
            Some(start) if time_ms >= start && time_ms - start <= MAX_BLOCK_TIMECODE => start,
            _ => {
                put_id(&mut self.out, id::CLUSTER);
                self.out.extend_from_slice(&UNKNOWN_SIZE);
                uint(&mut self.out, id::TIMECODE, time_ms);
                self.cluster = Some(time_ms);
                time_ms
            }
        };
        let offset = i16::try_from(time_ms - cluster).expect("kept within a cluster's range");
        let mut block = vec![0x81];
        block.extend_from_slice(&offset.to_be_bytes());
        block.push(0x80);
        block.extend_from_slice(packet);
        element(&mut self.out, id::SIMPLE_BLOCK, &block);
    }

    /// The file.
    pub fn finish(self) -> Vec<u8> {
        self.out
    }
}

/// A reader for the WebM this writes, for tests and checks: the EBML tree
/// as (id, payload) pairs, unknown sizes running to the end of their parent.
#[cfg(test)]
pub(crate) mod read {
    pub fn vint(data: &[u8]) -> Option<(u64, usize, bool)> {
        let first = *data.first()?;
        let len = first.leading_zeros() as usize + 1;
        if len > 8 || data.len() < len {
            return None;
        }
        let mut value = u64::from(first) & ((1u64 << (8 - len)) - 1);
        for &b in &data[1..len] {
            value = (value << 8) | u64::from(b);
        }
        let unknown = value == (1u64 << (7 * len)) - 1;
        Some((value, len, unknown))
    }

    pub fn id(data: &[u8]) -> Option<(u32, usize)> {
        let len = data.first()?.leading_zeros() as usize + 1;
        let mut value = 0u32;
        for &b in data.get(..len)? {
            value = (value << 8) | u32::from(b);
        }
        Some((value, len))
    }

    /// The elements at one level: (id, payload).
    pub fn children(data: &[u8]) -> Vec<(u32, &[u8])> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < data.len() {
            let (id, id_len) = id(&data[at..]).unwrap();
            let (size, size_len, unknown) = vint(&data[at + id_len..]).unwrap();
            let start = at + id_len + size_len;
            let end = if unknown {
                // Live mode: a Cluster runs to the next Cluster, the Segment to the end.
                let next = (start..data.len()).find(|&i| data[i..].starts_with(&[0x1F, 0x43, 0xB6, 0x75]));
                if id == super::id::CLUSTER {
                    next.unwrap_or(data.len())
                } else {
                    data.len()
                }
            } else {
                start + usize::try_from(size).unwrap()
            };
            out.push((id, &data[start..end]));
            at = end;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::read::children;
    use super::*;

    #[test]
    fn writes_sizes_in_the_fewest_bytes() {
        let size = |n| {
            let mut out = Vec::new();
            put_size(&mut out, n);
            out
        };
        assert_eq!(size(0), [0x80]);
        assert_eq!(size(126), [0xFE]);
        // 127 is all ones in one byte, which means unknown.
        assert_eq!(size(127), [0x40, 0x7F]);
        assert_eq!(size(16_382), [0x7F, 0xFE]);
        assert_eq!(size(16_383), [0x20, 0x3F, 0xFF]);
        let mut out = Vec::new();
        uint(&mut out, id::TRACK_UID, 0x00AB_CDEF_0123_4567);
        assert_eq!(out, [0x73, 0xC5, 0x87, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67]);
        let mut zero = Vec::new();
        uint(&mut zero, id::TIMECODE, 0);
        assert_eq!(zero, [0xE7, 0x81, 0x00]);
    }

    #[test]
    fn lays_out_the_file_as_chromium_does() {
        let mut writer = WebmWriter::new(1, 48_000, 0x0012_3456_789A_BCDE);
        writer.add_packet(0, &[1, 2, 3]);
        writer.add_packet(60, &[4]);
        writer.add_packet(32_760, &[5]);
        // Past 32 767 ms from the cluster's start: a new cluster.
        writer.add_packet(32_820, &[6]);
        let file = writer.finish();
        assert_eq!(&file[..4], [0x1A, 0x45, 0xDF, 0xA3]);

        let top = children(&file);
        assert_eq!(
            top.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [id::EBML, id::SEGMENT]
        );
        let header = children(top[0].1);
        assert_eq!(header[4], (id::DOC_TYPE, &b"webm"[..]));
        assert_eq!(header[5], (id::DOC_TYPE_VERSION, &[4u8][..]));
        assert_eq!(header[6], (id::DOC_TYPE_READ_VERSION, &[2u8][..]));

        let segment = children(top[1].1);
        let ids: Vec<u32> = segment.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [id::INFO, id::TRACKS, id::CLUSTER, id::CLUSTER]);
        let info = children(segment[0].1);
        assert_eq!(info[0], (id::TIMECODE_SCALE, &[0x0F, 0x42, 0x40][..]));
        assert_eq!(info[1], (id::MUXING_APP, &b"Chrome"[..]));
        let entry = children(children(segment[1].1)[0].1);
        let entry_ids: Vec<u32> = entry.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            entry_ids,
            [
                id::TRACK_NUMBER,
                id::TRACK_UID,
                id::TRACK_TYPE,
                id::CODEC_ID,
                id::CODEC_PRIVATE,
                id::AUDIO
            ]
        );
        assert_eq!(entry[3].1, b"A_OPUS");
        assert_eq!(
            entry[4].1,
            [
                b'O', b'p', b'u', b's', b'H', b'e', b'a', b'd', 1, 1, 0, 0, 0x80, 0xBB, 0, 0, 0, 0, 0
            ]
        );
        let audio = children(entry[5].1);
        assert_eq!(audio[0], (id::SAMPLING_FREQUENCY, &48_000f32.to_be_bytes()[..]));
        assert_eq!(audio[1], (id::CHANNELS, &[1u8][..]));
        assert_eq!(audio[2], (id::BIT_DEPTH, &[32u8][..]));

        let first = children(segment[2].1);
        assert_eq!(first[0], (id::TIMECODE, &[0u8][..]));
        assert_eq!(first[1], (id::SIMPLE_BLOCK, &[0x81, 0, 0, 0x80, 1, 2, 3][..]));
        assert_eq!(first[2], (id::SIMPLE_BLOCK, &[0x81, 0, 60, 0x80, 4][..]));
        assert_eq!(first[3].1[..3], [0x81, 0x7F, 0xF8]);
        let second = children(segment[3].1);
        assert_eq!(second[0], (id::TIMECODE, &[0x80, 0x34][..]));
        assert_eq!(second[1], (id::SIMPLE_BLOCK, &[0x81, 0, 0, 0x80, 6][..]));
    }
}
