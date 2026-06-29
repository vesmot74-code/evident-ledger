# PROOF_V1 SCHEMA (IMMUTABLE CONTRACT)

## Version
version: "proof_v1"

---

## Core structure

{
  version: "proof_v1",
  chain_id: string,
  head_event_id: string,

  events: [
    {
      event_id: string,
      sequence: number,
      parent_event_id: string,
      file_hash: string
    }
  ],

  proof: {
    root: string,
    chain_head: string,
    signature: string,
    public_key: string,
    leaves_count: number,
    type: "merkle-root-v1"
  }
}

---

## INVARIANTS

- events MUST be ordered by sequence ascending
- parent_event_id MUST reference previous event in causal chain
- file_hash MUST be deterministic hash of event payload
- root MUST be Merkle root over events
- signature MUST cover (root + chain_id + head_event_id)
- public_key MUST match signature verification key

---

## IMMUTABILITY RULE

Any modification to this schema requires:
→ version bump to proof_v2
→ backward compatibility is NOT required

proof_v1 is frozen permanently.
