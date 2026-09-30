use crate::domain::short_code::{ShortCode, ALPHABET};
use rand::Rng;

/// Produces candidate session codes. Uniqueness is the repository's job (retry on collision).
pub trait CodeGenerator: Send + Sync {
    fn generate(&self) -> ShortCode;
}

/// Uniformly random codes of a fixed length.
pub struct RandomCodeGenerator {
    length: usize,
}

impl RandomCodeGenerator {
    pub fn new(length: usize) -> Self {
        Self { length }
    }
}

impl CodeGenerator for RandomCodeGenerator {
    fn generate(&self) -> ShortCode {
        let mut rng = rand::thread_rng();
        ShortCode::from_alphabet_indices((0..self.length).map(|_| rng.gen_range(0..ALPHABET.len())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn generated_codes_have_configured_length_and_are_valid() {
        let g = RandomCodeGenerator::new(5);
        for _ in 0..500 {
            let code = g.generate();
            assert_eq!(code.as_str().len(), 5);
            assert!(ShortCode::parse(code.as_str(), 5).is_ok());
        }
    }

    #[test]
    fn length_follows_configuration() {
        assert_eq!(RandomCodeGenerator::new(8).generate().as_str().len(), 8);
    }

    #[test]
    fn codes_are_not_constant() {
        let g = RandomCodeGenerator::new(5);
        let set: HashSet<_> = (0..200).map(|_| g.generate()).collect();
        assert!(
            set.len() > 150,
            "expected mostly distinct codes, got {}",
            set.len()
        );
    }
}
