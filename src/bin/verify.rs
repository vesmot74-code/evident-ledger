use serde::Deserialize;
use std::process;

use evident_ledger::db::EventRow;
use evident_ledger::service::verification::check_event_structure;
use evident_ledger::signing::verify_root;

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

#[derive(Deserialize, Clone)]
struct EventLeaf {
    sequence: i64,
    event_id: String,
    parent_event_id: String,
    file_hash: String,
}

fn events_to_rows(events: &[EventLeaf]) -> Option<Vec<EventRow>> {
    use chrono::Utc;
    let mut rows = Vec::with_capacity(events.len());
    for event in events {
        rows.push(EventRow {
            event_id: uuid::Uuid::parse_str(&event.event_id).ok()?,
            parent_event_id: uuid::Uuid::parse_str(&event.parent_event_id).ok()?,
            file_hash: event.file_hash.clone(),
            created_at: Utc::now(),
            sequence: event.sequence,
        });
    }
    Some(rows)
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
            eprintln!(
                "FAIL: no pinned server key at {}",
                pinned_key_path.display()
            );
            eprintln!("Fetch it once: curl http://127.0.0.1:3000/identity");
            std::process::exit(3);
        }
    };

    let sig_valid = verify_root(
        &proof_file.chain_id,
        &proof_file.proof.root,
        &proof_file.proof.chain_head,
        &proof_file.proof.signature,
        &trusted_public_key,
    );
    if !sig_valid {
        eprintln!("FAIL: signature invalid (untrusted key or tampered data)");
        ok = false;
    }

    // 2. leaves_count
    if proof_file.events.len() != proof_file.proof.leaves_count {
        eprintln!("FAIL: leaves_count mismatch");
        ok = false;
    }

    // 3. head consistency
    if proof_file.head_event_id != proof_file.proof.chain_head {
        eprintln!("FAIL: head_event_id != chain_head");
        ok = false;
    }

    // 4–6. shared structural checks (parent, sequence, merkle)
    let rows = match events_to_rows(&proof_file.events) {
        Some(rows) => rows,
        None => {
            eprintln!("FAIL: invalid event UUIDs in proof");
            process::exit(2);
        }
    };

    match check_event_structure(&rows) {
        Ok(recomputed) => {
            if recomputed != proof_file.proof.root {
                eprintln!("FAIL: merkle root mismatch");
                eprintln!("  expected:   {}", proof_file.proof.root);
                eprintln!("  recomputed: {}", recomputed);
                ok = false;
            }
        }
        Err(evident_ledger::service::verification::StructuralFailure::Sequence { index }) => {
            eprintln!("FAIL: sequence not monotonic at index {index}");
            ok = false;
        }
        Err(evident_ledger::service::verification::StructuralFailure::ParentChain { index }) => {
            eprintln!("FAIL: parent mismatch at index {index}");
            ok = false;
        }
        Err(evident_ledger::service::verification::StructuralFailure::EmptyMerkle) => {
            eprintln!("FAIL: merkle root empty");
            ok = false;
        }
    }

    // Optional: original file check (informational only)
    let original_path = args.get(2);
    if let Some(original_path) = original_path {
        let maybe_ev = proof_file
            .events
            .iter()
            .find(|e| e.event_id == proof_file.head_event_id);
        let mut original_ok = false;
        if let Some(ev) = maybe_ev {
            if let Ok(bytes) = std::fs::read(original_path) {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(&bytes);
                let calc = format!("{:x}", hasher.finalize());
                if calc == ev.file_hash {
                    original_ok = true;
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
