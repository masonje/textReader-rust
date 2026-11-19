//! The Sec-MS-GEC token Microsoft's Read Aloud service requires, ported from the
//! edge-tts Python package (drm.py, v2 format "<CS64 hex>-<SHA-256 hex>").
//! The tests compare against values the Python package produced.

const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const WIN_EPOCH: f64 = 11_644_473_600.0;

const SHA256_INITIAL: [u32; 8] =
    [0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19];
// State after the first 14 rounds over Microsoft's hidden 57-byte prefix
const SHA256_STATE: [u32; 8] =
    [0xFD9FBEA2, 0x91243CE7, 0xBE499094, 0xCD2E1E0A, 0x6C006F49, 0x04BD35AB, 0xFEB0A7E4, 0x03EFFBD2];
const SHA256_SCHEDULE: [u32; 14] = [
    0xEB24BF9E, 0x1558691F, 0xA0C09EE2, 0x0CFA32BA, 0xA622EE8C, 0xD1DD9491, 0xC0DD25A0, 0xF9E70AB6, 0xF0C31573,
    0x7058CE4E, 0x7188E606, 0x00E5AB33, 0xD258B060, 0x72766564,
];
const K: [u32; 64] = [
    0x428A2F98, 0x71374491, 0xB5C0FBCF, 0xE9B5DBA5, 0x3956C25B, 0x59F111F1, 0x923F82A4, 0xAB1C5ED5, 0xD807AA98,
    0x12835B01, 0x243185BE, 0x550C7DC3, 0x72BE5D74, 0x80DEB1FE, 0x9BDC06A7, 0xC19BF174, 0xE49B69C1, 0xEFBE4786,
    0x0FC19DC6, 0x240CA1CC, 0x2DE92C6F, 0x4A7484AA, 0x5CB0A9DC, 0x76F988DA, 0x983E5152, 0xA831C66D, 0xB00327C8,
    0xBF597FC7, 0xC6E00BF3, 0xD5A79147, 0x06CA6351, 0x14292967, 0x27B70A85, 0x2E1B2138, 0x4D2C6DFC, 0x53380D13,
    0x650A7354, 0x766A0ABB, 0x81C2C92E, 0x92722C85, 0xA2BFE8A1, 0xA81A664B, 0xC24B8B70, 0xC76C51A3, 0xD192E819,
    0xD6990624, 0xF40E3585, 0x106AA070, 0x19A4C116, 0x1E376C08, 0x2748774C, 0x34B0BCB5, 0x391C0CB3, 0x4ED8AA4A,
    0x5B9CCA4F, 0x682E6FF3, 0x748F82EE, 0x78A5636F, 0x84C87814, 0x8CC70208, 0x90BEFFFA, 0xA4506CEB, 0xBEF9A3F7,
    0xC67178F2,
];
const ADJUSTMENT: u32 = 0xC9DC06A7; // replaces K[14] in the first block
const SIGMA0_ADJUSTMENT: u32 = 0x059C0B80;
const SIGMA1_ADJUSTMENT: u32 = 0x000B92C0;

const CS64_COEFFICIENTS: [(u32, u32); 14] = [
    (0xE82E91B1, 0xB32700D0),
    (0x44DE24C6, 0x88DFF51D),
    (0x3BEB65BC, 0x29514038),
    (0xC89C5933, 0xB328CBDF),
    (0x8D1EC211, 0xD405B4D5),
    (0x314EF5D5, 0x26E55F3C),
    (0xE92491B1, 0x607801C6),
    (0xF0A5B9D5, 0x82B2C1FB),
    (0x06064CF3, 0xFA51A3DA),
    (0xDA79FA60, 0x207CC464),
    (0xBED20560, 0x3C3C74F3),
    (0x89F9C9F8, 0xD2D14B61),
    (0x35FA38C6, 0xBB04A2C1),
    (0xCB49E3C6, 0xC5626167),
];
const CS64_FACTOR: u32 = 0x82893933;
const CS64_ADJUSTMENT0: u32 = 0xF2F90B1A;
const CS64_ADJUSTMENT1: u32 = 0x0B1A0000;

fn sigma0(v: u32) -> u32 {
    v.rotate_right(7) ^ v.rotate_right(18) ^ (v >> 3)
}
fn sigma1(v: u32) -> u32 {
    v.rotate_right(17) ^ v.rotate_right(19) ^ (v >> 10)
}

/// Standard SHA-256 rounds from `first_round`; the first block uses the adjusted constants.
fn rounds(state: [u32; 8], w: &[u32], first_round: usize) -> [u32; 8] {
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
    for i in first_round..64 {
        let k = if first_round == 14 && i == 14 { ADJUSTMENT } else { K[i] };
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = g ^ (e & (f ^ g));
        let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(k).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) | (c & (a | b));
        let t2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    [a, b, c, d, e, f, g, h]
}

fn expand(w: &mut Vec<u32>, first_word: usize) {
    for i in first_word..64 {
        let value = sigma1(w[i - 2]).wrapping_add(w[i - 7]).wrapping_add(sigma0(w[i - 15])).wrapping_add(w[i - 16]);
        w.push(value);
    }
}

fn be(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |acc, b| (acc << 8) | *b as u32)
}

fn sha256_init(data: &[u8]) -> [u32; 8] {
    let mut w = vec![0u32; 14];
    w.push(be(&data[..3]));
    w.push(be(&data[3..7]));
    for i in 16..30 {
        let mut value = SHA256_SCHEDULE[i - 16];
        if i - 2 >= 14 {
            let s = sigma1(w[i - 2]);
            value = value.wrapping_add(if i == 16 { s ^ SIGMA1_ADJUSTMENT } else { s });
        }
        if i - 7 >= 14 {
            value = value.wrapping_add(w[i - 7]);
        }
        if i - 15 >= 14 {
            let s = sigma0(w[i - 15]);
            value = value.wrapping_add(if i == 29 { s ^ SIGMA0_ADJUSTMENT } else { s });
        }
        w.push(value);
    }
    w.push(
        sigma1(w[28])
            .wrapping_add(w[23])
            .wrapping_add(sigma0(w[15]))
            .wrapping_add(w[14])
            .wrapping_add(ADJUSTMENT)
            .wrapping_sub(K[14]),
    );
    expand(&mut w, 31);
    let working = rounds(SHA256_STATE, &w, 14);
    std::array::from_fn(|i| SHA256_INITIAL[i].wrapping_add(working[i]))
}

fn sha256_digest(data: &[u8]) -> [u8; 32] {
    let length = 57 + data.len() as i64;
    let mut padded = data[7..].to_vec();
    padded.push(0x80);
    padded.extend(std::iter::repeat_n(0u8, (55 - length).rem_euclid(64) as usize));
    padded.extend(((length * 8) as u64).to_be_bytes());
    let mut state = sha256_init(&data[..7]);
    for block in padded.chunks(64) {
        let mut w: Vec<u32> = block.chunks(4).map(be).collect();
        expand(&mut w, 16);
        let working = rounds(state, &w, 0);
        state = std::array::from_fn(|i| state[i].wrapping_add(working[i]));
    }
    let mut out = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn rol16(v: u32) -> u32 {
    v.rotate_left(16)
}

fn words_le4_even(data: &[u8]) -> Vec<u32> {
    let mut padded = data.to_vec();
    while padded.len() % 4 != 0 {
        padded.push(0);
    }
    let mut words: Vec<u32> = padded.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if words.len() % 2 == 1 {
        words.push(0);
    }
    words
}

fn cs64_digest(data: &[u8], digest: &[u8; 32]) -> (u32, u32) {
    let key_a = u32::from_le_bytes(digest[0..4].try_into().unwrap()) | 1;
    let key_b = u32::from_le_bytes(digest[4..8].try_into().unwrap()) | 1;
    let factors = [key_a.wrapping_mul(CS64_FACTOR), key_b.wrapping_mul(CS64_FACTOR)];
    let mut terms: Vec<u32> = CS64_COEFFICIENTS.iter().enumerate().map(|(i, (c, _))| c.wrapping_mul(factors[i % 2])).collect();
    let mut offsets: Vec<u32> = CS64_COEFFICIENTS.iter().map(|(_, o)| *o).collect();
    let mut input = vec![0u8];
    input.extend_from_slice(data);
    for (i, word) in words_le4_even(&input).into_iter().enumerate() {
        let mut term = word.wrapping_mul(if i % 2 == 0 { key_a } else { key_b });
        let mut offset = rol16(word).wrapping_mul(if i % 2 == 0 { 0xEF0569FB } else { 0xC31713DB });
        if i == 0 {
            term = term.wrapping_add(CS64_ADJUSTMENT0.wrapping_mul(factors[0]));
            offset = offset.wrapping_add(CS64_ADJUSTMENT1);
        }
        terms.push(term);
        offsets.push(offset);
    }
    let (mut swap_acc, mut swap_last, mut state, mut acc) = (0u32, 0u32, 0u32, 0u32);
    for i in (0..terms.len()).step_by(2) {
        let (x, y) = (terms[i], terms[i + 1]);
        let t = rol16(offsets[i].wrapping_add(x));
        let q = t.wrapping_mul(0x76F50F34);
        swap_acc = swap_acc.wrapping_add(rol16(q).wrapping_mul(0xC3EFEA97).wrapping_add(q));
        let t = rol16(offsets[i + 1].wrapping_add(y));
        let q = t.wrapping_mul(0x1881CE3C);
        swap_last = rol16(q).wrapping_mul(0x35BD1EC9).wrapping_add(q);
        swap_acc = swap_acc.wrapping_add(swap_last);

        state = state.wrapping_mul(key_a).wrapping_add(x);
        state = rol16(state).wrapping_mul(0xCF98B111);
        state = rol16(state).wrapping_mul(0x87085B9F);
        state = rol16(state).wrapping_mul(0x12CEB96D);
        state = rol16(state).wrapping_mul(0x257E1D83);
        acc = acc.wrapping_add(state);
        let mut t = state.wrapping_mul(key_b).wrapping_add(y);
        t = rol16(t).wrapping_mul(0xA27416F5);
        t = rol16(t).wrapping_mul(0xD38396FF);
        state = rol16(rol16(t).wrapping_mul(0x7C932B89)).wrapping_mul(0xBFA49F69);
        acc = acc.wrapping_add(state);
    }
    (swap_last ^ state, swap_acc ^ acc)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// The token for a given time (seconds since 1970), valid for a 5-minute window.
pub fn sec_ms_gec(unix_time: f64) -> String {
    let mut ticks = unix_time + WIN_EPOCH;
    ticks -= ticks % 300.0;
    ticks *= 1e9 / 100.0;
    let data = format!("{ticks:.0}{TRUSTED_CLIENT_TOKEN}");
    let digest = sha256_digest(data.as_bytes());
    let (c0, c1) = cs64_digest(data.as_bytes(), &digest);
    let mut cs = c0.to_le_bytes().to_vec();
    cs.extend(c1.to_le_bytes());
    format!("{}-{}", hex(&cs), hex(&digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_python_edge_tts() {
        let cases = [
            (1700000000.0, "2CD23A2CA0B2595A-F3C846C370E05F9CC83ACC5F995C5F5D87A4D0F14CE8936A0F902E64099E38A2"),
            (1795076000.5, "0E190303672D2703-1268420ED8402D355843E1AFB8A0D4489F85D235E0D49D87C469927C2E93781A"),
            (1234567890.0, "BA405D1577B8E0CE-61F01E88539977A1A1E969572D82EECFA45FB1E5DEBD122D6ACBAF0054F959E5"),
        ];
        for (time, expected) in cases {
            assert_eq!(sec_ms_gec(time), expected, "time {time}");
        }
    }
}
