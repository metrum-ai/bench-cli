# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Early checks shared with actual runners for known case-specific inputs.

These checks are not an offline launch rehearsal: they do not cover every case,
assemble every runner argv, or parse every product SUT declaration.
"""
from pathlib import Path

LANES = {'BC-GPU-003': 'G1', 'BC-COMP-002': 'D', 'BC-GPU-011': 'G4',
         'BC-REASON-005': 'V', 'BC-GPU-013': 'G5', 'BC-GPU-014': 'G5', 'BC-GPU-015': 'G5'}


def check(case, config, candidate):
    if case in LANES:
        import r4_model_pins
        r4_model_pins.check(config, LANES[case], case)
    if case == 'BC-GPU-003':
        import g1_variants_runner
        g1_variants_runner.tool_prompt(config)
    elif case == 'BC-COMP-002':
        import comp_reference_runner
        comp_reference_runner.reference_inputs(config)
    elif case == 'BC-GPU-011':
        import g4_modality_runner
        g4_modality_runner.modality_inputs(config, Path(candidate))
    elif case == 'BC-REASON-005':
        import g4_modality_runner
        image = g4_modality_runner.pinned_file(config, 'vlm_image')
        prompt = g4_modality_runner.pinned_file(config['prompt_sets'], 'default')
        g4_modality_runner.image_prompts(prompt, image)
    elif case == 'BC-GPU-013':
        import g4_modality_runner
        g4_modality_runner.pinned_file(config, 'strategic_binary', 'metrum-ai-bench-cli-strategic', Path(candidate))
    elif case == 'BC-GPU-015':
        import g5_endpoints_runner
        g5_endpoints_runner.configuration(config)
