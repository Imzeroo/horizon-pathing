// The mod's row in Options > Mods.
fn pathing_listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    use crate::game::pathing as m;
    mods.push(crate::game::native_hooks::NativeMod {
        id: m::ID.into(),
        title: format!("Pathing {}", m::VERSION),
        description: "Walks to a place for you: /goto x y z, /path help.".into(),
        keys: Vec::new(),
    });
}
