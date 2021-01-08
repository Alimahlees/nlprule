//! Sets of grammatical error correction rules.

use crate::rule::{Cache, Rule};
use crate::tokenizer::Tokenizer;
use crate::types::*;
use crate::utils::parallelism::MaybeParallelRefIterator;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

/// Options for a rule set.
#[derive(Serialize, Deserialize, Clone)]
pub struct RulesOptions {
    /// Whether to allow errors while constructing the rules.
    pub allow_errors: bool,
    /// Grammar Rule IDs to use in this set.
    #[serde(default)]
    pub ids: Vec<String>,
    /// Grammar Rule IDs to ignore in this set.
    #[serde(default)]
    pub ignore_ids: Vec<String>,
}

impl Default for RulesOptions {
    fn default() -> Self {
        RulesOptions {
            allow_errors: true,
            ids: Vec::new(),
            ignore_ids: Vec::new(),
        }
    }
}

/// A set of grammatical error correction rules.
#[derive(Serialize, Deserialize, Default)]
pub struct Rules {
    rules: Vec<Rule>,
    cache: Cache,
}

impl Rules {
    /// Creates a rule set from a path to an XML file containing grammar rules.
    #[cfg(feature = "compile")]
    pub fn from_xml<P: AsRef<std::path::Path>>(path: P, options: RulesOptions) -> Self {
        use log::warn;
        use std::collections::HashMap;
        use std::convert::TryFrom;

        let rules = crate::rule::read_rules(path);
        let mut errors: HashMap<String, usize> = HashMap::new();

        let rules: Vec<_> = rules
            .into_iter()
            .filter_map(|x| match x {
                Ok((rule_structure, id, on)) => match Rule::try_from(rule_structure) {
                    Ok(mut rule) => {
                        if (options.ids.is_empty() || options.ids.contains(&id))
                            && !options.ignore_ids.contains(&id)
                        {
                            rule.set_id(id);
                            rule.set_on(on);
                            Some(rule)
                        } else {
                            None
                        }
                    }
                    Err(x) => {
                        *errors.entry(format!("[Rule] {}", x)).or_insert(0) += 1;
                        None
                    }
                },
                Err(x) => {
                    *errors.entry(format!("[Structure] {}", x)).or_insert(0) += 1;
                    None
                }
            })
            .collect();

        if !errors.is_empty() {
            let mut errors: Vec<(String, usize)> = errors.into_iter().collect();
            errors.sort_by_key(|x| -(x.1 as i32));

            warn!("Errors constructing Rules: {:#?}", &errors);
        }

        Rules {
            rules,
            cache: Cache::default(),
        }
    }

    /// Creates a new rules set from a file.
    pub fn new<P: AsRef<Path>>(p: P) -> bincode::Result<Self> {
        let reader = BufReader::new(File::open(p).unwrap());
        bincode::deserialize_from(reader)
    }

    /// Creates a new rules set from a reader.
    pub fn new_from<R: Read>(reader: R) -> bincode::Result<Self> {
        bincode::deserialize_from(reader)
    }

    /// Populates the cache of the rule set by checking whether the rules can match on a common set of words.
    pub fn populate_cache(&mut self, common_words: &HashSet<String>) {
        self.cache.populate(
            common_words,
            &self.rules.iter().map(|x| &x.engine).collect::<Vec<_>>(),
        );
    }

    pub fn rules(&self) -> &Vec<Rule> {
        &self.rules
    }

    /// Compute the suggestions for the given tokens by checking all rules.
    pub fn suggest(&self, tokens: &[Token], tokenizer: &Tokenizer) -> Vec<Suggestion> {
        if tokens.is_empty() {
            return Vec::new();
        }

        let mut output: Vec<Suggestion> = self
            .rules
            .maybe_par_iter()
            .enumerate()
            .filter(|(_, x)| x.on())
            .map(|(i, rule)| {
                let skip_mask = self.cache.get_skip_mask(tokens, i);
                let mut output = Vec::new();

                for suggestion in rule.apply(tokens, Some(&skip_mask), tokenizer) {
                    output.push(suggestion);
                }

                output
            })
            .flatten()
            .collect();

        output.sort_by(|a, b| a.start.cmp(&b.start));

        let mut mask = vec![false; tokens[0].text.chars().count()];
        output.retain(|suggestion| {
            if mask[suggestion.start..suggestion.end].iter().all(|x| !x) {
                mask[suggestion.start..suggestion.end]
                    .iter_mut()
                    .for_each(|x| *x = true);
                true
            } else {
                false
            }
        });

        output
    }
}

/// Correct a text by applying suggestions to it.
/// In the case of multiple possible replacements, always chooses the first one.
pub fn correct(text: &str, suggestions: &[Suggestion]) -> String {
    let mut offset: isize = 0;
    let mut chars: Vec<_> = text.chars().collect();

    for suggestion in suggestions {
        let replacement: Vec<_> = suggestion.text[0].chars().collect();
        chars.splice(
            (suggestion.start as isize + offset) as usize
                ..(suggestion.end as isize + offset) as usize,
            replacement.iter().cloned(),
        );
        offset = offset + replacement.len() as isize - (suggestion.end - suggestion.start) as isize;
    }

    chars.into_iter().collect()
}