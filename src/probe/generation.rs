// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Generation probes: mode 2 must produce source from the library's own generator, and the
//! target-capability query must answer from the factory rather than from a name list the designer
//! keeps (principle #12/#13: query the target, never infer from a name).
//!
//! The document below is a real project: a container, a label and a button. That is enough to
//! prove the generator parses a document, resolves the controls, and emits a named function — the
//! three things the designer's "generate" action depends on. It is not a claim that *every*
//! control generates; that coverage belongs to the library's own compile gate, and the designer's
//! per-control readiness is a separate, later measurement (principle #58).

use rust_widgets::designer::generator::validate_function_name;
use rust_widgets::designer::{
    Availability, GenerationRequest, TargetProfile, availability, generate,
};
use rust_widgets::platform::profile::{
    ProfileClass, full_widget_set, has_os_runtime, is_openharmony_target, profile_class,
    profile_name, stripped_widget_set,
};

use super::{ApiFamily, ProbeResult, require};

/// The family these probes belong to.
const FAMILY: ApiFamily = ApiFamily::Generation;

/// A small but complete project document, in the designer's saved format.
const DOCUMENT: &str = r#"{
  "window": {
    "id": "root",
    "title": "Probe",
    "width": 320,
    "height": 200,
    "layout": {
      "type": "vbox",
      "spacing": 4,
      "children": [
        { "label": { "id": "title", "text": "Hello" } },
        { "button": { "id": "go", "text": "Go" } }
      ]
    }
  }
}"#;

/// Runs every generation probe.
pub fn probes() -> Vec<ProbeResult> {
    vec![
        capability_query(),
        compiled_profile(),
        generate_default(),
        generate_stripped(),
        function_name_refusal(),
    ]
}

/// `platform::profile`: which profile the pinned dependency was actually compiled for.
///
/// # Why a target-capability probe asks this
///
/// `designer::availability` answers from the *running* build's factory, and the whole point of
/// `Availability::CrossProfileCaveat` is that the answer is only as good as the profile the
/// designer runs under (principle #12: query the target's own contract). The support matrix
/// (`docs/plan/support-matrix.json`) records which profile each row was established under, so the
/// build must be able to say which one it is — otherwise that row would be an assumption about the
/// build rather than a fact read from it.
fn compiled_profile() -> ProbeResult {
    const API: &str = "platform::profile";
    let name = profile_name();
    let class = profile_class();

    require!(
        name == "desktop",
        FAMILY,
        API,
        format!(
            "the pinned desktop build reports profile_name() == {name:?}, expected \"desktop\""
        )
    );
    require!(
        class == ProfileClass::Device,
        FAMILY,
        API,
        format!("the pinned build reports {class:?}, expected ProfileClass::Device")
    );
    require!(
        full_widget_set() && !stripped_widget_set(),
        FAMILY,
        API,
        format!(
            "the desktop profile reported full_widget_set() == {} and stripped_widget_set() == {}",
            full_widget_set(),
            stripped_widget_set()
        )
    );
    require!(
        has_os_runtime(),
        FAMILY,
        API,
        "the desktop profile reported no OS runtime to host a window"
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec![
            format!("profile_name -> {name:?}, class -> {class:?}"),
            format!(
                "full widget set -> {}, OS runtime -> {}, OpenHarmony target -> {}",
                full_widget_set(),
                has_os_runtime(),
                is_openharmony_target()
            ),
        ],
    )
}

/// `designer::availability`: the target answers from its own factory.
fn capability_query() -> ProbeResult {
    const API: &str = "designer::availability";
    let known = availability("button", TargetProfile::Default);
    let unknown = availability("definitely_not_a_widget", TargetProfile::Default);

    require!(
        matches!(known, Availability::Local),
        FAMILY,
        API,
        format!("`button` was reported {known:?}, expected Local")
    );
    require!(
        !matches!(unknown, Availability::Local),
        FAMILY,
        API,
        "an unresolvable widget name was reported as locally constructible; a name the factory \
         cannot build must never be reported as available"
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec![
            format!("button on desktop -> {known:?}"),
            format!("definitely_not_a_widget on desktop -> {unknown:?}"),
        ],
    )
}

/// `designer::generate` for the desktop/tablet/mobile template.
fn generate_default() -> ProbeResult {
    const API: &str = "designer::generate (Default)";
    let request = request(TargetProfile::Default, "probe_ui");

    let first = match generate(&request) {
        Ok(generated) => generated,
        Err(error) => {
            return ProbeResult::gap(FAMILY, API, format!("generation failed: {error}"));
        }
    };
    let second = match generate(&request) {
        Ok(generated) => generated,
        Err(error) => {
            return ProbeResult::gap(FAMILY, API, format!("second run failed: {error}"));
        }
    };

    require!(
        first.report.is_clean(),
        FAMILY,
        API,
        format!("the report carried gaps: {}", first.report.summary())
    );
    require!(
        first.source.contains("fn probe_ui"),
        FAMILY,
        API,
        "the emitted source does not contain the requested function"
    );
    require!(
        first.source == second.source,
        FAMILY,
        API,
        "two runs over one document produced different source; generation is not deterministic"
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec![
            format!("{} bytes emitted", first.source.len()),
            format!("report: {}", first.report.summary()),
        ],
    )
}

/// `designer::generate` for the mini/embedded template, which must not link the style engine.
///
/// # Why a clean report is *not* the expectation here
///
/// The probes run under the desktop profile, so generating for a `Stripped` target means the host
/// profile and the target differ. The library reports that honestly as
/// [`Availability::CrossProfileCaveat`], recorded in `cross_profile_caveats`. Treating that as a
/// failure would be wrong — it is the designed behaviour, and it is exactly the warning the
/// designer must surface to a user who generates for `mini` from a desktop — so this probe requires
/// what actually matters (nothing refused, nothing over capacity) and records the caveat count.
fn generate_stripped() -> ProbeResult {
    const API: &str = "designer::generate (Stripped)";
    let request = request(TargetProfile::Stripped, "probe_ui");

    match generate(&request) {
        Ok(generated) => {
            require!(
                generated.report.unsupported.is_empty(),
                FAMILY,
                API,
                format!(
                    "the stripped report refused something: {}",
                    generated.report.summary()
                )
            );
            require!(
                generated.report.capacity_overflow.is_empty(),
                FAMILY,
                API,
                format!(
                    "the stripped report overflowed a capacity: {}",
                    generated.report.summary()
                )
            );
            require!(
                generated.source.contains("fn probe_ui"),
                FAMILY,
                API,
                "the stripped source does not contain the requested function"
            );
            ProbeResult::verified(
                FAMILY,
                API,
                vec![
                    format!("{} bytes emitted", generated.source.len()),
                    format!("report: {}", generated.report.summary()),
                ],
            )
        }
        Err(error) => ProbeResult::gap(FAMILY, API, format!("generation failed: {error}")),
    }
}

/// `designer::generator::validate_function_name`: a bad name is refused before anything is emitted.
fn function_name_refusal() -> ProbeResult {
    const API: &str = "designer::generator::validate_function_name";
    let accepted = validate_function_name("ui_root");
    let refused = validate_function_name("123bad");

    require!(
        accepted.is_ok(),
        FAMILY,
        API,
        format!("`ui_root` was refused: {accepted:?}")
    );
    require!(
        refused.is_err(),
        FAMILY,
        API,
        "`123bad` was accepted as a function name; a name that cannot compile must be refused"
    );

    ProbeResult::verified(
        FAMILY,
        API,
        vec![format!("123bad -> {}", refused.expect_err("checked above"))],
    )
}

/// Builds a generation request over [`DOCUMENT`].
fn request(target: TargetProfile, function_name: &str) -> GenerationRequest {
    GenerationRequest {
        json: DOCUMENT.to_string(),
        target,
        width: 320,
        height: 200,
        function_name: function_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::probes;
    use crate::probe::ProbeOutcome;

    #[test]
    fn every_generation_probe_is_verified() {
        for probe in probes() {
            assert!(
                matches!(probe.outcome, ProbeOutcome::Verified),
                "generation probe {} was {:?}",
                probe.api,
                probe.outcome
            );
        }
    }
}
