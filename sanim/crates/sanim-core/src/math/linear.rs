//! Fixed-size, finite f64 linear algebra. Matrices are row-major.
use super::float_arithmetic as calc;

pub type Matrix4 = [[f64; 4]; 4];

/// Conservative local rectangle after the same stretch/scale/rotation/translation order as drawing.
pub fn transformed_box(
    bounds: [f64; 4],
    position: [f64; 2],
    rotation_degrees: f64,
    scale: f64,
    stretch: [f64; 2],
) -> Result<[f64; 4], String> {
    let (sin, cos) = rotation_degrees.to_radians().sin_cos();
    let mut output = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for x in [bounds[0], bounds[2]] {
        for y in [bounds[1], bounds[3]] {
            let x = calc(
                "*",
                calc(
                    "+",
                    stretch[1],
                    calc("*", calc("-", x, stretch[1])?, stretch[0])?,
                )?,
                scale,
            )?;
            let y = calc("*", y, scale)?;
            let point = [
                calc(
                    "+",
                    position[0],
                    calc("-", calc("*", x, cos)?, calc("*", y, sin)?)?,
                )?,
                calc(
                    "+",
                    position[1],
                    calc("+", calc("*", x, sin)?, calc("*", y, cos)?)?,
                )?,
            ];
            for axis in 0..2 {
                output[axis] = output[axis].min(point[axis]);
                output[axis + 2] = output[axis + 2].max(point[axis]);
            }
        }
    }
    Ok(output)
}

pub fn identity() -> Matrix4 {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    out
}

pub fn dot(a: &[f64], b: &[f64]) -> Result<f64, String> {
    if a.len() != b.len() {
        return Err("dot product dimensions must match".into());
    }
    a.iter()
        .zip(b)
        .try_fold(0.0, |sum, (a, b)| calc("+", sum, calc("*", *a, *b)?))
}

pub fn multiply(a: &Matrix4, b: &Matrix4) -> Result<Matrix4, String> {
    let bt = transpose(b);
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = dot(&a[i], &bt[j])?;
        }
    }
    Ok(out)
}

pub fn transform(matrix: &Matrix4, vector: &[f64]) -> Result<Vec<f64>, String> {
    if vector.len() != 4 {
        return Err(
            "Matrix4 multiplication requires Vec4; choose the homogeneous w component explicitly"
                .into(),
        );
    }
    matrix.iter().map(|row| dot(row, vector)).collect()
}

pub fn transpose(matrix: &Matrix4) -> Matrix4 {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = matrix[j][i];
        }
    }
    out
}

pub fn inverse(matrix: &Matrix4) -> Result<Matrix4, String> {
    let mut left = *matrix;
    let mut right = identity();
    for column in 0..4 {
        let pivot = (column..4)
            .max_by(|a, b| left[*a][column].abs().total_cmp(&left[*b][column].abs()))
            .filter(|row| left[*row][column] != 0.0)
            .ok_or("matrix is singular and has no inverse")?;
        left.swap(column, pivot);
        right.swap(column, pivot);
        let divisor = left[column][column];
        for j in 0..4 {
            left[column][j] = calc("/", left[column][j], divisor)?;
            right[column][j] = calc("/", right[column][j], divisor)?;
        }
        for row in 0..4 {
            if row == column {
                continue;
            }
            let factor = left[row][column];
            for j in 0..4 {
                left[row][j] = calc("-", left[row][j], calc("*", factor, left[column][j])?)?;
                right[row][j] = calc("-", right[row][j], calc("*", factor, right[column][j])?)?;
            }
        }
    }
    Ok(right)
}
