/// A quantity that may be ordered: positive, or the reason it is not.
pub fn check(n: u32) -> Result<u32, String> {
    if n == 0 {
        return Err("quantity must be positive".to_string());
    }
    Ok(n)
}
