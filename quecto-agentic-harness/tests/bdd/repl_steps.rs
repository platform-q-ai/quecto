use super::*;

fn execute_repl(world: &mut QuectoWorld) {
    if world.repl_executed {
        return;
    }
    world.repl_executed = true;
    let input = world.repl_input_lines.join("\n") + "\n";
    let output = cli::run_repl_with_output(&world.cli_context, &[], input.as_bytes(), true);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[when("I start quecto in REPL mode")]
fn when_start_repl(world: &mut QuectoWorld) {
    world.repl_input_lines.clear();
    world.repl_flags.clear();
    world.repl_executed = false;
}

#[when(expr = "I type {string}")]
fn when_type_line(world: &mut QuectoWorld, line: String) {
    let exits = matches!(line.as_str(), "exit" | "quit" | "/exit" | "/quit");
    world.repl_input_lines.push(line);
    if exits {
        execute_repl(world);
    }
}

#[when("I send EOF")]
fn when_send_eof(world: &mut QuectoWorld) {
    execute_repl(world);
}

#[then(expr = "stdout should not contain {string}")]
fn then_stdout_not_contains(world: &mut QuectoWorld, expected: String) {
    execute_repl(world);
    assert!(
        !world.stdout.contains(&expected),
        "stdout unexpectedly contained {expected:?}: {}",
        world.stdout
    );
}
