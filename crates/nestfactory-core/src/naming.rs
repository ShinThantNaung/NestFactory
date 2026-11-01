//! Name transforms matching `@angular-devkit/core` strings and the `pluralize` package, which
//! the Nest schematics use. Matching them exactly keeps generated names identical to the CLI.

use std::sync::OnceLock;

use regex::Regex;

pub fn decamelize(s: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"([a-z\d])([A-Z])").unwrap()).replace_all(s, "${1}_${2}").to_lowercase()
}

pub fn dasherize(s: &str) -> String {
    decamelize(s).replace([' ', '_'], "-")
}

pub fn camelize(s: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(-|_|\.|\s)+(.)?").unwrap());
    let out = re.replace_all(s, |c: &regex::Captures| c.get(2).map(|m| m.as_str().to_uppercase()).unwrap_or_default());
    let mut chars = out.chars();
    match chars.next() {
        Some(f) if f.is_ascii_uppercase() => f.to_lowercase().chain(chars).collect(),
        Some(f) => std::iter::once(f).chain(chars).collect(),
        None => String::new(),
    }
}

pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(f) => f.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub fn classify(s: &str) -> String {
    s.split('.').map(|p| capitalize(&camelize(p))).collect::<Vec<_>>().join(".")
}

pub fn underscore(s: &str) -> String {
    decamelize(s).replace([' ', '-'], "_")
}

/// `lowercased` helper from the resource schematic: classify, then lower the first letter.
pub fn lowercased(s: &str) -> String {
    let c = classify(s);
    let mut chars = c.chars();
    match chars.next() {
        Some(f) => f.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `normalizeToKebabOrSnakeCase` from the Nest schematics.
pub fn normalize_name(s: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"([a-z\d])([A-Z])").unwrap());
    re.replace_all(s.trim(), "${1}-${2}").to_lowercase().split_whitespace().collect::<Vec<_>>().join("-")
}

struct Rules {
    singular: Vec<(Regex, &'static str)>,
    irregular_plural_to_single: Vec<(&'static str, &'static str)>,
    uncountable_words: Vec<&'static str>,
    uncountable_rules: Vec<Regex>,
}

fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| {
        let rx = |s: &str| Regex::new(&format!("(?i){s}")).unwrap();
        let singular_src: &[(&str, &'static str)] = &[
            (r"s$", ""),
            (r"(ss)$", "$1"),
            (r"(wi|kni|(?:after|half|high|low|mid|non|night|[^\w]|^)li)ves$", "${1}fe"),
            (r"(ar|(?:wo|[ae])l|[eo][ao])ves$", "${1}f"),
            (r"ies$", "y"),
            (r"(dg|ss|ois|lk|ok|wn|mb|th|ch|ec|oal|is|ck|ix|sser|ts|wb)ies$", "${1}ie"),
            (r"\b(l|(?:neck|cross|hog|aun)?t|coll|faer|food|gen|goon|group|hipp|junk|vegg|(?:pork)?p|charl|calor|cut)ies$", "${1}ie"),
            (r"\b(mon|smil)ies$", "${1}ey"),
            (r"\b((?:tit)?m|l)ice$", "${1}ouse"),
            (r"(seraph|cherub)im$", "$1"),
            (r"(x|ch|ss|sh|zz|tto|go|cho|alias|[^aou]us|t[lm]as|gas|(?:her|at|gr)o|[aeiou]ris)(?:es)?$", "$1"),
            (r"(analy|diagno|parenthe|progno|synop|the|empha|cri|ne)(?:sis|ses)$", "${1}sis"),
            (r"(movie|twelve|abuse|e[mn]u)s$", "$1"),
            (r"(test)(?:is|es)$", "${1}is"),
            (r"(alumn|syllab|vir|radi|nucle|fung|cact|stimul|termin|bacill|foc|uter|loc|strat)(?:us|i)$", "${1}us"),
            (r"(agend|addend|millenni|dat|extrem|bacteri|desiderat|strat|candelabr|errat|ov|symposi|curricul|quor)a$", "${1}um"),
            (r"(apheli|hyperbat|periheli|asyndet|noumen|phenomen|criteri|organ|prolegomen|hedr|automat)a$", "${1}on"),
            (r"(alumn|alg|vertebr)ae$", "${1}a"),
            (r"(cod|mur|sil|vert|ind)ices$", "${1}ex"),
            (r"(matr|append)ices$", "${1}ix"),
            (r"(pe)(rson|ople)$", "${1}rson"),
            (r"(child)ren$", "$1"),
            (r"(eau)x?$", "$1"),
            (r"men$", "man"),
        ];
        let irregular: &[(&str, &str)] = &[
            ("I", "we"), ("me", "us"), ("he", "they"), ("she", "they"), ("them", "them"), ("myself", "ourselves"),
            ("yourself", "yourselves"), ("itself", "themselves"), ("herself", "themselves"), ("himself", "themselves"),
            ("themself", "themselves"), ("is", "are"), ("was", "were"), ("has", "have"), ("this", "these"), ("that", "those"),
            ("my", "our"), ("its", "their"), ("his", "their"), ("her", "their"), ("echo", "echoes"), ("dingo", "dingoes"),
            ("volcano", "volcanoes"), ("tornado", "tornadoes"), ("torpedo", "torpedoes"), ("genus", "genera"),
            ("viscus", "viscera"), ("stigma", "stigmata"), ("stoma", "stomata"), ("dogma", "dogmata"), ("lemma", "lemmata"),
            ("schema", "schemata"), ("anathema", "anathemata"), ("ox", "oxen"), ("axe", "axes"), ("die", "dice"),
            ("yes", "yeses"), ("foot", "feet"), ("eave", "eaves"), ("goose", "geese"), ("tooth", "teeth"), ("quiz", "quizzes"),
            ("human", "humans"), ("proof", "proofs"), ("carve", "carves"), ("valve", "valves"), ("looey", "looies"),
            ("thief", "thieves"), ("groove", "grooves"), ("pickaxe", "pickaxes"), ("passerby", "passersby"),
        ];
        let uncountable_words = vec![
            "adulthood", "advice", "agenda", "aid", "aircraft", "alcohol", "ammo", "analytics", "anime", "athletics",
            "audio", "bison", "blood", "bream", "buffalo", "butter", "carp", "cash", "chassis", "chess", "clothing", "cod",
            "commerce", "cooperation", "corps", "debris", "diabetes", "digestion", "elk", "energy", "equipment",
            "excretion", "expertise", "firmware", "flounder", "fun", "gallows", "garbage", "graffiti", "hardware",
            "headquarters", "health", "herpes", "highjinks", "homework", "housework", "information", "jeans", "justice",
            "kudos", "labour", "literature", "machinery", "mackerel", "mail", "media", "mews", "moose", "music", "mud",
            "manga", "news", "only", "personnel", "pike", "plankton", "pliers", "police", "pollution", "premises", "rain",
            "research", "rice", "salmon", "scissors", "series", "sewage", "shambles", "shrimp", "software", "staff",
            "swine", "tennis", "traffic", "transportation", "trout", "tuna", "wealth", "welfare", "whiting",
            "wildebeest", "wildlife", "you",
        ];
        let uncountable_rules = [r"pok[eé]mon$", r"[^aeiou]ese$", r"deer$", r"fish$", r"measles$", r"o[iu]s$", r"pox$", r"sheep$"]
            .iter()
            .map(|s| rx(s))
            .collect();
        Rules {
            singular: singular_src.iter().map(|(r, rep)| (rx(r), *rep)).collect(),
            irregular_plural_to_single: irregular.iter().map(|(s, p)| (*p, *s)).collect(),
            uncountable_words,
            uncountable_rules,
        }
    })
}

fn restore_case(word: &str, token: &str) -> String {
    if word == token {
        return token.to_string();
    }
    if word == word.to_lowercase() {
        return token.to_lowercase();
    }
    if word == word.to_uppercase() {
        return token.to_uppercase();
    }
    if word.chars().next().is_some_and(|c| c.is_uppercase()) {
        return capitalize(&token.to_lowercase());
    }
    token.to_lowercase()
}

/// `pluralize.singular`.
pub fn singular(word: &str) -> String {
    let r = rules();
    let token = word.to_lowercase();
    if word.is_empty() || r.uncountable_words.contains(&token.as_str()) || r.uncountable_rules.iter().any(|re| re.is_match(&token)) {
        return word.to_string();
    }
    if let Some((_, single)) = r.irregular_plural_to_single.iter().find(|(p, _)| *p == token) {
        return restore_case(word, single);
    }
    // Already-singular irregulars stay as they are.
    if r.irregular_plural_to_single.iter().any(|(_, s)| *s == token) {
        return word.to_string();
    }
    for (re, rep) in r.singular.iter().rev() {
        if let Some(m) = re.captures(word) {
            let whole = m.get(0).unwrap();
            let mut replaced = String::new();
            m.expand(rep, &mut replaced);
            let replaced = if whole.as_str().is_empty() {
                let prev = &word[..whole.start()];
                restore_case(prev.chars().last().map(|c| c.to_string()).unwrap_or_default().as_str(), &replaced)
            } else {
                restore_case(whole.as_str(), &replaced)
            };
            return format!("{}{}{}", &word[..whole.start()], replaced, &word[whole.end()..]);
        }
    }
    word.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings() {
        assert_eq!(classify("invoice-items"), "InvoiceItems");
        assert_eq!(classify("create-invoice.dto"), "CreateInvoice.Dto");
        assert_eq!(dasherize("InvoiceItems"), "invoice-items");
        assert_eq!(camelize("invoice-items"), "invoiceItems");
        assert_eq!(lowercased("invoice-items"), "invoiceItems");
        assert_eq!(normalize_name("InvoiceItems"), "invoice-items");
        assert_eq!(normalize_name("user profile"), "user-profile");
    }

    #[test]
    fn singulars() {
        for (p, s) in [
            ("invoices", "invoice"),
            ("Invoices", "Invoice"),
            ("users", "user"),
            ("categories", "category"),
            ("addresses", "address"),
            ("status", "status"),
            ("people", "person"),
            ("children", "child"),
            ("news", "news"),
            ("boxes", "box"),
            ("InvoiceItems", "InvoiceItem"),
            ("data", "datum"),
            ("orders", "order"),
        ] {
            assert_eq!(singular(p), s, "singular({p})");
        }
    }
}
