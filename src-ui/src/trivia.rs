/// Facts about vanilla Minecraft, shown a couple at a time beside the release
/// notes. Kept to things that are true of the game as Mojang ships it — no mod,
/// launcher or server specifics.
const TRIVIA: &[&str] = &[
    "Creepers came from a mistake. Notch swapped the height and length while \
     modelling a pig, and shipped the tall green result instead of fixing it.",
    "Before it had a name, Minecraft was called Cave Game.",
    "A full day-night cycle lasts 20 real-world minutes, of which 10 are daylight.",
    "A player walks at roughly 4.3 blocks per second, and sprinting is about a \
     third faster again.",
    "Endermen take damage from water, so an ordinary rain shower will drive them \
     to teleport away.",
    "\"Removed Herobrine\" has appeared in patch notes for years. Herobrine has \
     never been in the game.",
    "Since 1.18 a world runs from Y -64 to Y 320 — 384 blocks tall, half again \
     what it was before.",
    "Diamonds are most common around Y -59, and get rarer in every direction \
     from there.",
    "Cats keep creepers and phantoms away, which makes them worth more than \
     their nine lives around a base.",
    "The crafting table was called the workbench for years, and plenty of \
     players still call it that.",
    "Most items stack to 64, but ender pearls stop at 16 and a bucket will not \
     stack at all.",
    "Sheep regrow their wool by eating grass, so a shorn flock on dirt stays \
     shorn.",
];

/// Pick `count` distinct facts at random. Returns every fact when `count` meets
/// or exceeds how many there are, so the caller cannot ask for an impossible
/// draw and hang.
pub fn pick(count: usize) -> Vec<&'static str> {
    if count >= TRIVIA.len() {
        return TRIVIA.to_vec();
    }
    let mut chosen: Vec<usize> = Vec::with_capacity(count);
    while chosen.len() < count {
        let index = (js_sys::Math::random() * TRIVIA.len() as f64) as usize;
        // `Math::random` is exclusive of 1.0, but clamp rather than trust the
        // float multiply at the top of the range.
        let index = index.min(TRIVIA.len() - 1);
        if !chosen.contains(&index) {
            chosen.push(index);
        }
    }
    chosen.into_iter().map(|index| TRIVIA[index]).collect()
}

#[cfg(test)]
mod tests {
    use super::{pick, TRIVIA};

    #[test]
    fn every_fact_is_present_and_readable() {
        assert!(TRIVIA.len() >= 2, "too few facts to draw from");
        for fact in TRIVIA {
            assert!(!fact.trim().is_empty());
            // Continuation lines are joined by the escape, not left ragged.
            assert!(!fact.contains("  "), "double space in: {fact}");
        }
    }

    #[test]
    fn asking_for_more_than_exist_returns_them_all() {
        assert_eq!(pick(TRIVIA.len() + 5).len(), TRIVIA.len());
    }
}
