use std::process;
use serde::Deserialize;
use sha2::{Sha256, Digest};

#[derive(Deserialize)]
struct ProofFile {
    chain_id: String,
    head_event_id: String,
    proof: Proof,
    events: Vec<EventLeaf>,
}

#[derive(Deserialize)]
struct Proof {
    root: String,
    chain_head: String,
    signature: String,
    public_key: String,
    leaves_count: usize,
}

#[derive(Deserialize)]
struct EventLeaf {
    sequence: i64,
    event_id: String,
    parent_event_id: String,
    file_hash: String,
}

fn build_leaf(sequence: i64, parent_event_id: &str, file_hash: &str) -> String {
    let hex: String = parent_event_id.replace('-', "");
    let parent_bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i+2], 16).unwrap_or(0))
        .collect();
    let mut hasher = Sha256::new();
    hasher.update(sequence.to_be_bytes());
    hasher.update(&parent_bytes);
    hasher.update(file_hash.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn recompute_root(events: &[EventLeaf]) -> String {
    if events.is_empty() { return "empty".to_string(); }

    let leaves: Vec<String> = events.iter()
        .map(|e| build_leaf(e.sequence, &e.parent_event_id, &e.file_hash))
        .collect();

    if leaves.len() == 1 { return leaves[0].clone(); }

    // server hashes each leaf again before building tree
    let mut hashed: Vec<String> = leaves.iter().map(|leaf| {
        let mut hasher = Sha256::new();
        hasher.update(leaf.as_bytes());
        format!("{:x}", hasher.finalize())
    }).collect();

    while hashed.len() > 1 {
        let mut next = Vec::new();
        for chunk in hashed.chunks(2) {
            let left = &chunk[0];
            let right = if chunk.len() > 1 { &chunk[1] } else { left };
            let mut hasher = Sha256::new();
            hasher.update(left.as_bytes());
            hasher.update(right.as_bytes());
            next.push(format!("{:x}", hasher.finalize()));
        }
        hashed = next;
    }
    hashed[0].clone()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: evident-verify <proof.json>");
        process::exit(1);
    }

    let content = std::fs::read_to_string(&args[1]).expect("Cannot read file");
    let proof_file: ProofFile = serde_json::from_str(&content).expect("Invalid JSON");

    let mut ok = true;

    // 1. signature
let pinned_key_path = dirs::home_dir()
    .expect("no home dir")
    .join(".evident")
    .join("server_identity.pub");
let trusted_public_key = match std::fs::read_to_string(&pinned_key_path) {
    Ok(k) => k.trim().to_string(),
    Err(_) => {
        eprintln!("FAIL: no pinned server key at {}", pinned_key_path.display());
        eprintln!("Fetch it once: curl http://127.0.0.1:3000/identity");
        std::process::exit(3);
    }
};

let sig_valid = evident_ledger::signing::verify_root(
    &proof_file.chain_id,
    &proof_file.proof.root,
    &proof_file.proof.chain_head,
    &proof_file.proof.signature,
    &trusted_public_key,
);
if !sig_valid { eprintln!("FAIL: signature invalid (untrusted key or tampered data)"); ok = false; }

    // 2. leaves_count
    if proof_file.events.len() != proof_file.proof.leaves_count {
        eprintln!("FAIL: leaves_count mismatch"); ok = false;
    }

    // 3. head consistency
    if proof_file.head_event_id != proof_file.proof.chain_head {
        eprintln!("FAIL: head_event_id != chain_head"); ok = false;
    }

    // 4. sequence monotonic
    for i in 1..proof_file.events.len() {
        if proof_file.events[i].sequence != proof_file.events[i-1].sequence + 1 {
            eprintln!("FAIL: sequence not monotonic at index {}", i); ok = false;
        }
    }

    // 5. parent chain
    for i in 1..proof_file.events.len() {
        if proof_file.events[i].parent_event_id != proof_file.events[i-1].event_id {
            eprintln!("FAIL: parent mismatch at index {}", i); ok = false;
        }
    }

    // 6. merkle recompute
    let recomputed = recompute_root(&proof_file.events);
    if recomputed != proof_file.proof.root {
        eprintln!("FAIL: merkle root mismatch");
        eprintln!("  expected:   {}", proof_file.proof.root);
        eprintln!("  recomputed: {}", recomputed);
        ok = false;
    }

    // Optional: original file check (informational only)
    let original_path = args.get(2);
    if let Some(original_path) = original_path {
        // find the event with event_id == head_event_id
        let maybe_ev = proof_file
            .events
            .iter()
            .find(|e| e.event_id == proof_file.head_event_id);
        let mut original_ok = false;
        if let Some(ev) = maybe_ev {
            match std::fs::read(original_path) {
                Ok(bytes) => {
                    let mut hasher = Sha256::new();
                    hasher.update(&bytes);
                    let calc = format!("{:x}", hasher.finalize());
                    if calc == ev.file_hash {
                        original_ok = true;
                    }
                }
                Err(_) => {
                    original_ok = false;
                }
            }
        }
        if original_ok {
            println!("Original: OK");
        } else {
            println!("Original: MISSING or MISMATCH");
        }
    }

    if ok {
        println!("OK: proof valid");
    } else {
        println!("FAIL");
        process::exit(2);
    }
}
