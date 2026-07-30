use ed25519_dalek::SigningKey;
use std::fs;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: print_key_pub <signing_key.bin>");

    let bytes = fs::read(path).expect("cannot read key");

    let array: [u8; 32] = bytes.try_into().expect("key must be exactly 32 bytes");

    let signing_key = SigningKey::from_bytes(&array);

    println!("{}", hex::encode(signing_key.verifying_key().to_bytes()));
}
