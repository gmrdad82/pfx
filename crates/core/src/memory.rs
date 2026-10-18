use std::collections::TryReserveError;

pub fn refusal(what: &str, bytes: u128, error: &TryReserveError) -> String {
    format!("{what} needs {bytes} bytes: {error}")
}

pub fn bytes_of<T>(len: usize) -> u128 {
    len as u128 * size_of::<T>().max(1) as u128
}

pub fn vec<T>(len: usize, what: &str) -> Result<Vec<T>, String> {
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|error| refusal(what, bytes_of::<T>(len), &error))?;
    Ok(out)
}

pub fn filled<T: Clone>(len: usize, value: T, what: &str) -> Result<Vec<T>, String> {
    let mut out = vec(len, what)?;
    out.resize(len, value);
    Ok(out)
}

pub fn more<T>(out: &mut Vec<T>, extra: usize, what: &str) -> Result<(), String> {
    out.try_reserve(extra)
        .map_err(|error| refusal(what, bytes_of::<T>(out.len() + extra), &error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_past_the_address_space_is_refused_with_its_size() {
        let error = vec::<[f32; 4]>(usize::MAX / 2, "a frame").unwrap_err();
        assert!(
            error.starts_with(&format!(
                "a frame needs {} bytes: ",
                (usize::MAX / 2) as u128 * 16
            )),
            "{error}"
        );
        assert_eq!(filled(3, 7u8, "three").unwrap(), [7, 7, 7]);
    }
}
