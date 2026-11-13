//! A mob's armour and toughness as its damage sees them (`LivingEntity.getArmorValue`,
//! `getAttributeValue(ARMOR_TOUGHNESS)`), for native mods that show them (the registry's Jade, as
//! newer Horizon's `mobs::armor::values` gives them). The same sums `Mob::hurt_after_armor`
//! makes: the kind's own, the worn pieces' and a closed shulker shell's, clamped as the
//! attributes are.
use super::{Mob, Species};

/// Armour (floored, as `getArmorValue` converts) and toughness.
pub fn values(mob: &Mob) -> (f32, f32) {
    let (worn, worn_toughness) = mob.worn_armor();
    let covered = if mob.species() == Species::Shulker && mob.extra.counter == 0 {
        20.0
    } else {
        0.0
    };
    let armor = (mob.kind.armor + worn + covered).clamp(0.0, 30.0);
    let toughness = (mob.kind.armor_toughness + worn_toughness).clamp(0.0, 20.0);
    (armor.floor(), toughness)
}
