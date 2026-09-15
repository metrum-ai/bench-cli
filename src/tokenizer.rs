// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Optional local tokenizer counts. Builds without the `tokenizer` feature
//! remain lightweight and continue to report server usage only.

#[derive(Debug)]
pub enum LocalTokenizer {
    #[cfg(feature = "tokenizer")]
    HuggingFace(Box<tokenizers::Tokenizer>),
    Disabled,
}

impl LocalTokenizer {
    pub fn from_file(path: Option<&str>) -> anyhow::Result<Self> {
        let Some(path) = path else {
            return Ok(Self::Disabled);
        };
        #[cfg(feature = "tokenizer")]
        {
            tokenizers::Tokenizer::from_file(path)
                .map(Box::new)
                .map(Self::HuggingFace)
                .map_err(|error| anyhow::anyhow!("load tokenizer {path}: {error}"))
        }
        #[cfg(not(feature = "tokenizer"))]
        {
            anyhow::bail!(
                "--tokenizer requires a build with `--features tokenizer` (requested {path})"
            )
        }
    }

    pub fn count(&self, text: &str) -> anyhow::Result<Option<u64>> {
        match self {
            #[cfg(feature = "tokenizer")]
            Self::HuggingFace(tokenizer) => tokenizer
                .encode(text, false)
                .map(|encoding| Some(encoding.len() as u64))
                .map_err(|error| anyhow::anyhow!("tokenize text: {error}")),
            Self::Disabled => {
                let _ = text;
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_tokenizer_has_no_count() {
        let tokenizer = LocalTokenizer::from_file(None).unwrap();
        assert_eq!(tokenizer.count("hello").unwrap(), None);
    }
}
