use super::{validate_capability, AggregateSnapshot};
use crate::diagnostic::DiagnosticCode;
use crate::effect::EffectStatus;
use crate::identity::{AggregateId, Capability, MachineIdentity, MachineSpecHash, MachineVersion};
use serde_json::Value;

// Every ASCII string of length 0..=4, including embedded NUL.
fn bounded_nonce() -> String {
    let len: usize = kani::any();
    kani::assume(len <= 4);
    let bytes: [u8; 4] = kani::any();
    let ascii = [
        bytes[0] & 0x7f,
        bytes[1] & 0x7f,
        bytes[2] & 0x7f,
        bytes[3] & 0x7f,
    ];
    let mut nonce = String::with_capacity(4);
    for byte in ascii {
        nonce.push(char::from(byte));
    }
    nonce.truncate(len);
    nonce
}

fn snapshot(generation: u64, nonce: String) -> AggregateSnapshot {
    AggregateSnapshot {
        aggregate_id: AggregateId(String::new()),
        machine: MachineIdentity {
            machine_id: String::new(),
            version: MachineVersion(0),
            spec_hash: MachineSpecHash(String::new()),
        },
        state: Value::Null,
        terminal: false,
        generation,
        nonce,
        effect: EffectStatus::None,
        sequence: 0,
        last_event_hash: String::new(),
        last_receipt_id: String::new(),
    }
}

#[kani::proof]
#[kani::unwind(24)]
fn capability_contract() {
    let current_generation: u64 = kani::any();
    let supplied_generation: u64 = kani::any();
    let snapshot = snapshot(current_generation, bounded_nonce());
    let capability = Capability {
        generation: supplied_generation,
        nonce: bounded_nonce(),
    };
    let same_generation = current_generation == supplied_generation;
    let same_nonce = snapshot.nonce == capability.nonce;
    kani::cover!(same_generation && same_nonce, "matching pair reachable");
    kani::cover!(same_generation && !same_nonce, "nonce mismatch reachable");
    kani::cover!(
        !same_generation && same_nonce,
        "generation mismatch reachable"
    );
    kani::cover!(!same_generation && !same_nonce, "both mismatches reachable");

    let result = validate_capability(&snapshot, &capability);
    match result {
        Ok(()) => assert!(same_generation && same_nonce, "only matching pair accepted"),
        Err(diagnostic) => {
            if !same_generation {
                assert!(matches!(diagnostic.code, DiagnosticCode::IkCapabilityStale));
                assert!(diagnostic.rule.as_deref() == Some("capability.generation"));
            } else {
                assert!(!same_nonce, "matching pair must be accepted");
                assert!(matches!(diagnostic.code, DiagnosticCode::IkNonceInvalid));
                assert!(diagnostic.rule.as_deref() == Some("capability.nonce"));
            }
        }
    }
}

#[kani::proof]
#[kani::unwind(24)]
fn intentionally_wrong_matching_is_rejected() {
    let generation: u64 = kani::any();
    let snapshot = snapshot(generation, String::new());
    let capability = Capability {
        generation,
        nonce: String::new(),
    };
    let result = validate_capability(&snapshot, &capability);
    assert!(
        result.is_err(),
        "deliberately false: matching pair rejected"
    );
}

mod authority;
