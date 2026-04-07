const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Encode a byte slice as base58 into the provided buffer.
/// Returns the number of bytes written, or None if the buffer is too small.
pub fn encode_base58(input: &[u8], buf: &mut [u8]) -> Option<usize> {
    // Count leading zeros
    let leading_zeros = input.iter().take_while(|&&b| b == 0).count();

    // Work buffer for division — max 32 bytes input → max 44 base58 digits
    let mut digits = [0u8; 64];
    let mut digit_len = 0usize;

    for &byte in &input[leading_zeros..] {
        let mut carry = byte as u32;
        let mut j = 0;
        while j < digit_len || carry > 0 {
            if j < digit_len {
                carry += (digits[j] as u32) << 8;
            }
            digits[j] = (carry % 58) as u8;
            carry /= 58;
            j += 1;
        }
        digit_len = j;
    }

    let total_len = leading_zeros + digit_len;
    if total_len > buf.len() {
        return None;
    }

    // Leading '1's for zero bytes
    let mut pos = 0;
    while pos < leading_zeros {
        buf[pos] = b'1';
        pos += 1;
    }

    // Base58 digits in reverse order
    let mut i = digit_len;
    while i > 0 {
        i -= 1;
        buf[pos] = ALPHABET[digits[i] as usize];
        pos += 1;
    }

    Some(pos)
}
