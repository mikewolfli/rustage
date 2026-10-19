// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A replaceable business process built from the designer's own modules.
//!
//! The fixture exists so `--logic-candidate build:<manifest>` can compile a *real* second artifact
//! and show that an admitted candidate is independent of the running binary. The three modules are
//! included from the designer rather than copied: one definition of the core, the channel and the
//! worker, so the candidate cannot drift into speaking another protocol.

mod app;

fn main() {
    use app::logic::core::Transition;
    use app::logic::migration::{FieldType, Schema};
    use app::logic::worker::{Program, Worker, serve};

    // A different program from the generation it replaces: `idle --go--> running` is the same, but
    // the new logic adds `running --approve--> done`. The driver restores the old state (`running`)
    // and then sends `approve`; only the new artifact can reach `done` from there.
    //
    // The schema is the generation's own contract, not the driver's: a candidate whose parent sends
    // a field this build does not declare (or a value of the wrong type) must refuse the import
    // rather than hold a state it cannot reduce.
    let schema = Schema::new(2).with("count", FieldType::Number, 1);
    let mut program = Program::new("orders", "0.2.0", "idle").with_schema(schema);
    for transition in [
        Transition::new("idle", "go", "running"),
        Transition::new("running", "approve", "done"),
    ] {
        program = match program.with(transition) {
            Ok(program) => program,
            Err(refusal) => {
                eprintln!("logic-candidate: {}", refusal.render());
                std::process::exit(2);
            }
        };
    }

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    if let Err(error) = serve(Worker::new(program), &mut input, &mut output) {
        eprintln!("logic-candidate: {}", error.render());
        std::process::exit(2);
    }
}
