use ed25519_dalek::SigningKey;
use std::fs;

fn check(path: &str) {
    let bytes = fs::read(path).expect("cannot read key");
    let array: [u8; 32] = bytes.try_into().expect("key must be 32 bytes");

    let signing_key = SigningKey::from_bytes(&array);
    let public_key = signing_key.verifying_key();

    println!("FILE:");
    println!("{}", path);
    println!("PUBLIC KEY:");
    println!("{}", hex::encode(public_key.to_bytes()));
    println!();
}

fn main() {
    check("./signing_key.bin");
    check("target/pilot116-key.JBOhAH/signing_key.bin");
}
