/*M///////////////////////////////////////////////////////////////////////////////////////
//
//  IMPORTANT: READ BEFORE DOWNLOADING, COPYING, INSTALLING OR USING.
//
//  By downloading, copying, installing or using the software you agree to this license.
//  If you do not agree to this license, do not download, install,
//  copy or use the software.
//
//
//                          License Agreement
//                For Open Source Computer Vision Library
//
// Copyright (C) 2000-2008, Intel Corporation, all rights reserved.
// Copyright (C) 2009, Willow Garage Inc., all rights reserved.
// Copyright (C) 2013, OpenCV Foundation, all rights reserved.
// Third party copyrights are property of their respective owners.
//
// Redistribution and use in source and binary forms, with or without modification,
// are permitted provided that the following conditions are met:
//
//   * Redistribution's of source code must retain the above copyright notice,
//     this list of conditions and the following disclaimer.
//
//   * Redistribution's in binary form must reproduce the above copyright notice,
//     this list of conditions and the following disclaimer in the documentation
//     and/or other materials provided with the distribution.
//
//   * The name of the copyright holders may not be used to endorse or promote products
//     derived from this software without specific prior written permission.
//
// This software is provided by the copyright holders and contributors "as is" and
// any express or implied warranties, including, but not limited to, the implied
// warranties of merchantability and fitness for a particular purpose are disclaimed.
// In no event shall the Intel Corporation or contributors be liable for any direct,
// indirect, incidental, special, exemplary, or consequential damages
// (including, but not limited to, procurement of substitute goods or services;
// loss of use, data, or profits; or business interruption) however caused
// and on any theory of liability, whether in contract, strict liability,
// or tort (including negligence or otherwise) arising in any way out of
// the use of this software, even if advised of the possibility of such damage.
//
//M*/

/*
   This is a translation to C++ from the Matlab's LMSolve package by Miroslav Balda.
   Here is the original copyright:
   ============================================================================

   Copyright (c) 2007, Miroslav Balda
   All rights reserved.

   Redistribution and use in source and binary forms, with or without
   modification, are permitted provided that the following conditions are
   met:

       * Redistributions of source code must retain the above copyright
         notice, this list of conditions and the following disclaimer.
       * Redistributions in binary form must reproduce the above copyright
         notice, this list of conditions and the following disclaimer in
         the documentation and/or other materials provided with the distribution

   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
   AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
   IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
   ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
   LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
   CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
   SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
   INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
   CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
   ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
   POSSIBILITY OF SUCH DAMAGE.
*/

//! OpenCV 4.13.0 source port of the affine estimator used by stabilization.
//!
//! Source: modules/calib3d/src/ptsetreg.cpp:55–254, 602–700, 833–887,
//! 1128–1205; precomp.hpp:117–139; levmarq.cpp:89–211;
//! modules/core/src/lapack.cpp:115–258, 613–673;
//! modules/core/include/opencv2/core/operations.hpp:409–419.
//! Only the three-point, float-input affine RANSAC and its six-parameter
//! Levenberg–Marquardt refinement are retained. Matrix storage is fixed-size;
//! OpenCV's arithmetic precision, random sequence and stopping rules are kept.

const MODEL_POINTS: usize = 3;
const PARAMETERS: usize = 6;
const SUBSET_ATTEMPTS: usize = 10_000;
const RNG_COEFFICIENT: u64 = 4_164_903_690;
const JACOBI_ITERATIONS_PER_ELEMENT: usize = 30;
const LM_LOW_RATIO: f64 = 0.25;
const LM_HIGH_RATIO: f64 = 0.75;
const LM_INITIAL_LAMBDA: f64 = 1.0;
const LM_INITIAL_CUTOFF: f64 = 0.75;

type Matrix = [[f64; PARAMETERS]; PARAMETERS];
type Parameters = [f64; PARAMETERS];

#[derive(Clone, Copy, Debug)]
pub struct AffineOptions {
    pub reprojection_threshold: f64,
    pub maximum_iterations: usize,
    pub confidence: f64,
    pub refinement_iterations: usize,
}

pub fn estimate_affine(
    from: &[[f32; 2]],
    to: &[[f32; 2]],
    options: AffineOptions,
) -> Result<[[f64; 3]; 2], String> {
    if from.len() != to.len() || from.len() < MODEL_POINTS {
        return Err("affine estimation needs at least three matching points".into());
    }
    if !(0.0 < options.confidence && options.confidence < 1.0) {
        return Err("affine RANSAC confidence must be between zero and one".into());
    }
    let count = from.len();
    let mut best = [0.0; PARAMETERS];
    let mut best_mask = vec![false; count];
    if count == MODEL_POINTS {
        best = affine_kernel(from, to);
        best_mask.fill(true);
    } else {
        // RANSACPointSetRegistrator::run constructs RNG((uint64)-1).
        let mut state = u64::MAX;
        let mut maximum_good = 0;
        let mut iterations = options.maximum_iterations.max(1);
        let mut iteration = 0;
        while iteration < iterations {
            let mut subset = [0; MODEL_POINTS];
            let mut source = [[0.0; 2]; MODEL_POINTS];
            let mut destination = [[0.0; 2]; MODEL_POINTS];
            let mut found = false;
            for _ in 0..SUBSET_ATTEMPTS {
                for i in 0..MODEL_POINTS {
                    loop {
                        state = u64::from(state as u32) * RNG_COEFFICIENT + (state >> 32);
                        let index = (state as u32 as usize) % count;
                        if !subset[..i].contains(&index) {
                            subset[i] = index;
                            source[i] = from[index];
                            destination[i] = to[index];
                            break;
                        }
                    }
                }
                if !collinear(&source) && !collinear(&destination) {
                    found = true;
                    break;
                }
            }
            if !found {
                if iteration == 0 {
                    return Err("affine RANSAC found no non-collinear subset".into());
                }
                break;
            }
            let model = affine_kernel(&source, &destination);
            let float_model = model.map(|value| value as f32);
            let threshold =
                (options.reprojection_threshold * options.reprojection_threshold) as f32;
            let mut good = 0;
            let mask = from
                .iter()
                .zip(to)
                .map(|(f, t)| {
                    // Affine2DEstimatorCallback::computeError intentionally uses floats.
                    let a = float_model[0] * f[0] + float_model[1] * f[1] + float_model[2] - t[0];
                    let b = float_model[3] * f[0] + float_model[4] * f[1] + float_model[5] - t[1];
                    let inlier = a * a + b * b <= threshold;
                    good += usize::from(inlier);
                    inlier
                })
                .collect::<Vec<_>>();
            if good > maximum_good.max(MODEL_POINTS - 1) {
                best = model;
                best_mask = mask;
                maximum_good = good;
                iterations = ransac_iterations(
                    options.confidence,
                    (count - good) as f64 / count as f64,
                    iterations,
                );
            }
            iteration += 1;
        }
        if maximum_good == 0 {
            return Err("affine RANSAC found no model".into());
        }
    }
    if count > MODEL_POINTS && options.refinement_iterations != 0 {
        let source = from
            .iter()
            .zip(&best_mask)
            .filter_map(|(p, valid)| valid.then_some(*p))
            .collect::<Vec<_>>();
        let destination = to
            .iter()
            .zip(&best_mask)
            .filter_map(|(p, valid)| valid.then_some(*p))
            .collect::<Vec<_>>();
        refine(
            &source,
            &destination,
            &mut best,
            options.refinement_iterations,
        );
    }
    Ok([[best[0], best[1], best[2]], [best[3], best[4], best[5]]])
}

fn ransac_iterations(confidence: f64, outliers: f64, maximum: usize) -> usize {
    let numerator = (1.0 - confidence.clamp(0.0, 1.0)).max(f64::MIN_POSITIVE);
    let denominator = 1.0 - (1.0 - outliers.clamp(0.0, 1.0)).powi(MODEL_POINTS as i32);
    if denominator < f64::MIN_POSITIVE {
        return 0;
    }
    let numerator = numerator.ln();
    let denominator = denominator.ln();
    if denominator >= 0.0 || -numerator >= maximum as f64 * -denominator {
        maximum
    } else {
        (numerator / denominator).round_ties_even() as usize
    }
}

fn collinear(points: &[[f32; 2]; MODEL_POINTS]) -> bool {
    let i = MODEL_POINTS - 1;
    for j in 0..i {
        // The C++ subtractions happen in float before assignment to double.
        let dx1 = f64::from(points[j][0] - points[i][0]);
        let dy1 = f64::from(points[j][1] - points[i][1]);
        for k in 0..j {
            let dx2 = f64::from(points[k][0] - points[i][0]);
            let dy2 = f64::from(points[k][1] - points[i][1]);
            if (dx2 * dy1 - dy2 * dx1).abs()
                <= f64::from(f32::EPSILON) * (dx1.abs() + dy1.abs() + dx2.abs() + dy2.abs())
            {
                return true;
            }
        }
    }
    false
}

fn affine_kernel(from: &[[f32; 2]], to: &[[f32; 2]]) -> Parameters {
    let [x1, y1] = from[0].map(f64::from);
    let [x2, y2] = from[1].map(f64::from);
    let [x3, y3] = from[2].map(f64::from);
    let d = 1.0 / (x1 * (y2 - y3) + x2 * (y3 - y1) + x3 * (y1 - y2));
    let mut result = [0.0; PARAMETERS];
    for axis in 0..2 {
        let a = f64::from(to[0][axis]);
        let b = f64::from(to[1][axis]);
        let c = f64::from(to[2][axis]);
        result[axis * 3] = d * (a * (y2 - y3) + b * (y3 - y1) + c * (y1 - y2));
        result[axis * 3 + 1] = d * (a * (x3 - x2) + b * (x1 - x3) + c * (x2 - x1));
        result[axis * 3 + 2] =
            d * (a * (x2 * y3 - x3 * y2) + b * (x3 * y1 - x1 * y3) + c * (x1 * y2 - x2 * y1));
    }
    result
}

// Affine2DRefineCallback::compute followed by mulTransposed(J), J.t()*r.
fn normal_equations(
    from: &[[f32; 2]],
    to: &[[f32; 2]],
    x: &Parameters,
) -> (Matrix, Parameters, Vec<f64>) {
    let mut a = [[0.0; PARAMETERS]; PARAMETERS];
    let mut v = [0.0; PARAMETERS];
    let mut residual = Vec::with_capacity(from.len() * 2);
    for (source, destination) in from.iter().zip(to) {
        let [mx, my] = source.map(f64::from);
        let row = [mx, my, 1.0];
        for (axis, destination) in destination.iter().enumerate() {
            let offset = axis * 3;
            let error =
                x[offset] * mx + x[offset + 1] * my + x[offset + 2] - f64::from(*destination);
            residual.push(error);
            for j in 0..3 {
                v[offset + j] += row[j] * error;
                for k in 0..3 {
                    a[offset + j][offset + k] += row[j] * row[k];
                }
            }
        }
    }
    (a, v, residual)
}

fn refine(from: &[[f32; 2]], to: &[[f32; 2]], x: &mut Parameters, maximum: usize) {
    let (mut a, mut v, mut r) = normal_equations(from, to, x);
    let mut s = r.iter().map(|value| value * value).sum::<f64>();
    let diagonal = std::array::from_fn::<_, PARAMETERS, _>(|i| a[i][i]);
    let mut lambda = LM_INITIAL_LAMBDA;
    let mut cutoff = LM_INITIAL_CUTOFF;
    for _ in 0..maximum {
        let mut ap = a;
        for i in 0..PARAMETERS {
            ap[i][i] += lambda * diagonal[i];
        }
        let (eigenvalues, eigenvectors) = jacobi(ap);
        let d = eigen_solve(&eigenvalues, &eigenvectors, &v);
        let xd = std::array::from_fn(|i| x[i] - d[i]);
        let (ad, vd, rd) = normal_equations(from, to, &xd);
        let sd = rd.iter().map(|value| value * value).sum::<f64>();
        let temp = std::array::from_fn::<_, PARAMETERS, _>(|i| {
            2.0 * v[i] - a[i].iter().zip(d).map(|(a, d)| a * d).sum::<f64>()
        });
        let ds = d.iter().zip(temp).map(|(d, t)| d * t).sum::<f64>();
        let ratio = (s - sd) / if ds.abs() > f64::EPSILON { ds } else { 1.0 };
        if ratio > LM_HIGH_RATIO {
            lambda *= 0.5;
            if lambda < cutoff {
                lambda = 0.0;
            }
        } else if ratio < LM_LOW_RATIO {
            let t = d.iter().zip(v).map(|(d, v)| d * v).sum::<f64>();
            let mut nu =
                ((sd - s) / if t.abs() > f64::EPSILON { t } else { 1.0 } + 2.0).clamp(2.0, 10.0);
            if lambda == 0.0 {
                let (values, vectors) = jacobi(a);
                let mut maximum = f64::EPSILON;
                for i in 0..PARAMETERS {
                    let mut unit = [0.0; PARAMETERS];
                    unit[i] = 1.0;
                    maximum = maximum.max(eigen_solve(&values, &vectors, &unit)[i].abs());
                }
                cutoff = 1.0 / maximum;
                lambda = cutoff;
                nu *= 0.5;
            }
            lambda *= nu;
        }
        if sd < s {
            s = sd;
            *x = xd;
            a = ad;
            v = vd;
            r = rd;
        }
        let step_norm = d.iter().fold(0.0_f64, |norm, value| norm.max(value.abs()));
        let error_norm = r.iter().fold(0.0_f64, |norm, value| norm.max(value.abs()));
        if step_norm < f64::from(f32::EPSILON) || error_norm < f64::from(f32::EPSILON) {
            break;
        }
    }
}

// JacobiImpl_<double>, including cached row/column maxima and pivot tie rules.
fn jacobi(mut a: Matrix) -> (Parameters, Matrix) {
    let mut w = std::array::from_fn(|i| a[i][i]);
    let mut v = [[0.0; PARAMETERS]; PARAMETERS];
    let mut rows = [0; PARAMETERS];
    let mut columns = [0; PARAMETERS];
    for k in 0..PARAMETERS {
        v[k][k] = 1.0;
        if k + 1 < PARAMETERS {
            rows[k] = k + 1;
            for i in k + 2..PARAMETERS {
                if a[k][rows[k]].abs() < a[k][i].abs() {
                    rows[k] = i;
                }
            }
        }
        if k > 0 {
            for i in 1..k {
                if a[columns[k]][k].abs() < a[i][k].abs() {
                    columns[k] = i;
                }
            }
        }
    }
    for _ in 0..PARAMETERS * PARAMETERS * JACOBI_ITERATIONS_PER_ELEMENT {
        let mut k = 0;
        let mut magnitude = a[0][rows[0]].abs();
        for i in 1..PARAMETERS - 1 {
            let value = a[i][rows[i]].abs();
            if magnitude < value {
                magnitude = value;
                k = i;
            }
        }
        let mut l = rows[k];
        for i in 1..PARAMETERS {
            let value = a[columns[i]][i].abs();
            if magnitude < value {
                magnitude = value;
                k = columns[i];
                l = i;
            }
        }
        let p = a[k][l];
        if p.abs() <= f64::EPSILON {
            break;
        }
        let y = (w[l] - w[k]) * 0.5;
        // OpenCV's local hypot scales to avoid intermediate overflow.
        let mut t = y.abs() + stable_hypot(p, y);
        let mut sine = stable_hypot(p, t);
        let cosine = t / sine;
        sine = p / sine;
        t = (p / t) * p;
        if y < 0.0 {
            sine = -sine;
            t = -t;
        }
        a[k][l] = 0.0;
        w[k] -= t;
        w[l] += t;
        for row in a.iter_mut().take(k) {
            let (a0, b0) = (row[k], row[l]);
            row[k] = a0 * cosine - b0 * sine;
            row[l] = a0 * sine + b0 * cosine;
        }
        let (before_l, from_l) = a.split_at_mut(l);
        let (through_k, after_k) = before_l.split_at_mut(k + 1);
        let pivot_row = &mut through_k[k];
        for (offset, row) in after_k.iter_mut().enumerate() {
            let i = k + 1 + offset;
            let (a0, b0) = (pivot_row[i], row[l]);
            pivot_row[i] = a0 * cosine - b0 * sine;
            row[l] = a0 * sine + b0 * cosine;
        }
        for (first, second) in pivot_row[l + 1..].iter_mut().zip(&mut from_l[0][l + 1..]) {
            let (a0, b0) = (*first, *second);
            *first = a0 * cosine - b0 * sine;
            *second = a0 * sine + b0 * cosine;
        }
        let (before_l, from_l) = v.split_at_mut(l);
        for (first, second) in before_l[k].iter_mut().zip(&mut from_l[0]) {
            let (a0, b0) = (*first, *second);
            *first = a0 * cosine - b0 * sine;
            *second = a0 * sine + b0 * cosine;
        }
        for index in [k, l] {
            if index + 1 < PARAMETERS {
                rows[index] = index + 1;
                for i in index + 2..PARAMETERS {
                    if a[index][rows[index]].abs() < a[index][i].abs() {
                        rows[index] = i;
                    }
                }
            }
            if index > 0 {
                columns[index] = 0;
                for i in 1..index {
                    if a[columns[index]][index].abs() < a[i][index].abs() {
                        columns[index] = i;
                    }
                }
            }
        }
    }
    for k in 0..PARAMETERS - 1 {
        let mut maximum = k;
        for i in k + 1..PARAMETERS {
            if w[maximum] < w[i] {
                maximum = i;
            }
        }
        if k != maximum {
            w.swap(k, maximum);
            v.swap(k, maximum);
        }
    }
    (w, v)
}

fn stable_hypot(a: f64, b: f64) -> f64 {
    let a = a.abs();
    let b = b.abs();
    if a > b {
        let ratio = b / a;
        a * (1.0 + ratio * ratio).sqrt()
    } else if b > 0.0 {
        let ratio = a / b;
        b * (1.0 + ratio * ratio).sqrt()
    } else {
        0.0
    }
}

// SVBkSbImpl_<double> with u == v and both stored transposed, one RHS.
fn eigen_solve(w: &Parameters, v: &Matrix, rhs: &Parameters) -> Parameters {
    let threshold = w.iter().sum::<f64>() * (f64::EPSILON * 2.0);
    let mut result = [0.0; PARAMETERS];
    for i in 0..PARAMETERS {
        if w[i].abs() <= threshold {
            continue;
        }
        let mut s = 0.0;
        for (coefficient, value) in v[i].iter().zip(rhs) {
            s += coefficient * value;
        }
        s *= 1.0 / w[i];
        for (value, coefficient) in result.iter_mut().zip(v[i]) {
            *value += s * coefficient;
        }
    }
    result
}

mod rotation;
pub use rotation::rotate_rgba;
