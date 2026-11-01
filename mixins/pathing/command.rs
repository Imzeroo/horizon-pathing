// /goto and /path, before the server sees them.
fn pathing_command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    crate::game::pathing_glue::command(game, command, ran);
}
