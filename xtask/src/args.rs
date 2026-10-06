//! A minimal command line parser: positionals, `--flag`s and `--name value` options.

use anyhow::{Result, bail};

#[derive(Debug)]
pub struct Args {
    words: Vec<String>,
}

impl Args {
    pub fn from_env() -> Args {
        Args {
            words: std::env::args().skip(1).collect(),
        }
    }

    /// Removes and returns the first word that is not an option.
    pub fn next_positional(&mut self) -> Option<String> {
        let index = self.words.iter().position(|w| !w.starts_with("--"))?;
        Some(self.words.remove(index))
    }

    /// Removes `name` if present.
    pub fn flag(&mut self, name: &str) -> bool {
        match self.words.iter().position(|w| w == name) {
            Some(index) => {
                self.words.remove(index);
                true
            }
            None => false,
        }
    }

    /// Removes `name value` and returns the value, if present.
    pub fn value(&mut self, name: &str) -> Result<Option<String>> {
        let Some(index) = self.words.iter().position(|w| w == name) else {
            return Ok(None);
        };
        if index + 1 >= self.words.len() {
            bail!("missing value for {name}");
        }
        let value = self.words.remove(index + 1);
        self.words.remove(index);
        Ok(Some(value))
    }

    /// Removes every `name value` pair and returns the values in order.
    pub fn values(&mut self, name: &str) -> Result<Vec<String>> {
        let mut values = Vec::new();
        while let Some(value) = self.value(name)? {
            values.push(value);
        }
        Ok(values)
    }

    /// Everything not consumed yet, verbatim.
    pub fn rest(self) -> Vec<String> {
        self.words
    }

    /// Fails if anything was not consumed.
    pub fn finish(self) -> Result<()> {
        if !self.words.is_empty() {
            bail!("unexpected arguments: {}", self.words.join(" "));
        }
        Ok(())
    }
}
