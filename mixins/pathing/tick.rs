// Plans and walks the current /goto, once a tick.
fn pathing_tick(game: &mut crate::game::Game<'_>) {
    crate::game::pathing_glue::tick(game);
}
