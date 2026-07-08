use crate::bundle::repo::BundleRepo;
use crate::bundle::store::StoreResult;
use crate::bundle::types::ValidationResult;

pub struct Validator;

impl Validator {
    pub fn validate_bundle(repo: &BundleRepo) -> StoreResult<ValidationResult> {
        repo.validate()
    }
}
