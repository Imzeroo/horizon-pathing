// Tells the server's command tree about /goto and /path, so the chat box
// completes their arguments (single player, and a LAN world you host).
fn pathing_register(
    game: &crate::game::Game<'_>,
    commands: &mut Vec<crate::game::native_server::NativeCommand>,
) {
    use crate::game::pathing as m;
    if game.native_on(m::ID) {
        for usage in m::USAGES {
            commands.push(crate::game::native_server::NativeCommand::new(usage));
        }
    }
}