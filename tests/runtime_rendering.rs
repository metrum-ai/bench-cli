// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
#![cfg(feature = "tokenizer")]

use metrum_ai_bench::chat_rendering::{RenderLimits, RENDERER_VERSION};
use metrum_ai_bench::prefix_validation::RenderingIdentity;
use metrum_ai_bench::runtime_rendering::{prepare_wire_body, PinnedRenderer, TokenizeCrossCheck};
use metrum_ai_bench::token_overlap::{validate_prefix_disjoint, Phase, Verdict};
use serde_json::json;
use sha2::{Digest, Sha256};

fn sha(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hex = String::new();
    for byte in Sha256::digest(bytes) {
        write!(&mut hex, "{byte:02x}").unwrap();
    }
    hex
}

#[test]
fn exact_wire_ownership_and_optional_cross_check_fail_closed_without_exporting_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let tok = dir.path().join("tokenizer.json");
    let tpl = dir.path().join("chat.jinja");
    let tokenizer = br#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":null,"post_processor":null,"decoder":null,"model":{"type":"BPE","dropout":null,"unk_token":null,"continuing_subword_prefix":null,"end_of_word_suffix":null,"fuse_unk":false,"byte_fallback":false,"vocab":{"a":0,"b":1},"merges":[]}}"#;
    let template = b"{% for message in messages %}{{ message.content }}{% endfor %}";
    std::fs::write(&tok, tokenizer).unwrap();
    std::fs::write(&tpl, template).unwrap();
    let identity = RenderingIdentity {
        tokenizer_id: "fixture/bpe".into(),
        tokenizer_revision: "a".repeat(40),
        tokenizer_sha256: sha(tokenizer),
        chat_template_sha256: sha(template),
        add_special_tokens: false,
        add_generation_prompt: true,
        hint_nonce_included: false,
        renderer_version: RENDERER_VERSION.into(),
    };
    let special_tokens = json!({});
    let config = PinnedRenderer {
        tokenizer_path: &tok,
        template_path: &tpl,
        identity: &identity,
        special_tokens: &special_tokens,
        require_server_cross_check: false,
        limits: RenderLimits {
            max_artifact_bytes: 8192,
            max_body_bytes: 8192,
            max_rendered_bytes: 128,
            max_tokens: 128,
            template_fuel: 10000,
        },
    };
    let warm_wire = br#" { "messages" : [{"role":"user","content":"a"}], "metadata":{"private":"KEEP-PRIVATE"} } "#.to_vec();
    let mut old_identity = identity.clone();
    old_identity.renderer_version = "minijinja-chat.v1".into();
    assert_ne!(old_identity.renderer_version, RENDERER_VERSION);
    let old_config = PinnedRenderer {
        identity: &old_identity,
        ..config
    };
    assert!(
        prepare_wire_body("old".into(), Phase::Warmup, warm_wire.clone(), &old_config).is_err()
    );
    let warm = prepare_wire_body("w".into(), Phase::Warmup, warm_wire.clone(), &config).unwrap();
    assert_eq!(warm.wire_bytes(), warm_wire);
    assert_eq!(warm.evidence().body_sha256, sha(&warm_wire));
    assert_eq!(warm.evidence().rendered_text_sha256, sha(b"a"));
    let compact =
        serde_json::to_vec(&serde_json::from_slice::<serde_json::Value>(&warm_wire).unwrap())
            .unwrap();
    let other = prepare_wire_body("w2".into(), Phase::Warmup, compact, &config).unwrap();
    assert_ne!(warm.evidence().body_sha256, other.evidence().body_sha256);
    assert_eq!(
        warm.evidence().rendered_text_sha256,
        other.evidence().rendered_text_sha256
    );
    let measured_wire = br#"{"messages":[{"role":"user","content":"b"}]}"#.to_vec();
    let mut measured =
        prepare_wire_body("m".into(), Phase::Measured, measured_wire.clone(), &config).unwrap();
    assert_eq!(
        validate_prefix_disjoint(&[warm.for_overlap()], &[measured.for_overlap()], 1).verdict,
        Verdict::Pass
    );
    let required = PinnedRenderer {
        require_server_cross_check: true,
        ..config
    };
    let mut pending = prepare_wire_body(
        "p".into(),
        Phase::Measured,
        measured_wire.clone(),
        &required,
    )
    .unwrap();
    assert_eq!(
        pending.evidence().server_tokenize_cross_check,
        TokenizeCrossCheck::Pending
    );
    assert_eq!(
        validate_prefix_disjoint(&[warm.for_overlap()], &[pending.for_overlap()], 1).verdict,
        Verdict::Unknown
    );
    pending.record_server_token_ids(Some(&[1]));
    assert_eq!(
        validate_prefix_disjoint(&[warm.for_overlap()], &[pending.for_overlap()], 1).verdict,
        Verdict::Pass
    );
    measured.record_server_token_ids(Some(&[1]));
    assert_eq!(
        measured.evidence().server_tokenize_cross_check,
        TokenizeCrossCheck::Match
    );
    measured.record_server_token_ids(Some(&[0]));
    assert_eq!(
        validate_prefix_disjoint(&[warm.for_overlap()], &[measured.for_overlap()], 1).verdict,
        Verdict::Unknown
    );
    measured.record_server_token_ids(Some(&[1]));
    assert_eq!(
        measured.evidence().server_tokenize_cross_check,
        TokenizeCrossCheck::Mismatch
    );
    assert_eq!(measured.wire_bytes(), measured_wire);
    let evidence = serde_json::to_value(measured.evidence()).unwrap();
    assert!(evidence.get("token_ids").is_none());
    assert!(!serde_json::to_string(warm.evidence())
        .unwrap()
        .contains("KEEP-PRIVATE"));
    let mut unavailable =
        prepare_wire_body("u".into(), Phase::Measured, measured_wire.clone(), &config).unwrap();
    unavailable.record_server_token_ids(None);
    unavailable.record_server_token_ids(Some(&[1]));
    assert_eq!(
        unavailable.evidence().server_tokenize_cross_check,
        TokenizeCrossCheck::Unavailable
    );
    assert_eq!(
        validate_prefix_disjoint(&[warm.for_overlap()], &[unavailable.for_overlap()], 1).verdict,
        Verdict::Unknown
    );
    std::fs::write(&tpl, b"tampered").unwrap();
    assert!(prepare_wire_body("bad".into(), Phase::Measured, measured_wire, &config).is_err());
}
