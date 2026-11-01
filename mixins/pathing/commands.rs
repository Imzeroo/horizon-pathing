// Tells the chat box about /goto and /path, so it completes them.
fn pathing_commands(game: &crate::game::Game<'_>, commands: &mut Vec<&'static str>) {
    crate::game::pathing_glue::commands(game, commands);
}
