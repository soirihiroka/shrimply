//! Intrinsic image sizing uses the language's existing CSS-pixel unit definition.
use super::{Fraction, Number, arithmetic};
pub fn intrinsic_size(source: [u32; 2]) -> Result<[Fraction; 2], String> {
    Ok([
        Number::parse(&format!("{}px", source[0]))?.value,
        Number::parse(&format!("{}px", source[1]))?.value,
    ])
}
pub fn aspect_size(
    source: [u32; 2],
    axis: usize,
    length: Fraction,
) -> Result<[Fraction; 2], String> {
    if axis > 1 || source.contains(&0) {
        return Err(
            "image aspect sizing requires a valid axis and positive source dimensions".into(),
        );
    }
    let mut size = [length; 2];
    size[1 - axis] = arithmetic(
        "*",
        length,
        Fraction::new(u64::from(source[1 - axis]), u64::from(source[axis])),
    )?;
    Ok(size)
}
