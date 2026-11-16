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

//! OpenCV 5.1.0 source port of the affine estimator used by stabilization.
//!
//! Source: modules/geometry/src/ptsetreg.cpp:54–253, 601–699, 965–1094;
//! precomp.hpp:141–163; levmarq.cpp:48–377, 484–725;
//! modules/geometry/include/opencv2/geometry/3d.hpp:535–699;
//! modules/core/src/lapack.cpp:92–107, 343–540, 609–670, 1282–1317;
//! modules/core/include/opencv2/core/operations.hpp:402–425.
//! Only the three-point, float-input affine RANSAC and its six-parameter
//! default geodesic Levenberg–Marquardt refinement are retained. Matrix storage is fixed-size;
//! OpenCV's arithmetic precision, random sequence and stopping rules are kept.

const MODEL_POINTS: usize = 3;
const PARAMETERS: usize = 6;
const SUBSET_ATTEMPTS: usize = 10_000;
const RNG_COEFFICIENT: u64 = 4_164_903_690;
const SVD_MINIMUM_SWEEPS: usize = 30;
const SVD_ZERO_ATTEMPTS: usize = 100;
const SVD_EPSILON: f64 = f64::EPSILON * 10.0;
const LM_INITIAL_LAMBDA: f64 = 1e-4;
const LM_INITIAL_UP_FACTOR: f64 = 2.0;
const LM_DOWN_FACTOR: f64 = 3.0;
const LM_MINIMUM_DIAGONAL: f64 = 1e-6;
const LM_MAXIMUM_DIAGONAL: f64 = 1e32;
const LM_MAXIMUM_LAMBDA: f64 = 1e32;
const LM_TOLERANCE: f64 = 1e-6;
const LM_GEODESIC_STEP: f64 = 1e-4;
const LM_GEODESIC_SCALE: f64 = 0.5;

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

// estimateAffine2D's refinement callback followed by mulTransposed(J), J.t()*r.
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

// LevMarqBase::optimize with estimateAffine2D's default dense, unmasked settings.
fn refine(from: &[[f32; 2]], to: &[[f32; 2]], x: &mut Parameters, maximum: usize) {
    let (mut a, mut gradient, residual) = normal_equations(from, to, x);
    let mut energy = residual.iter().map(|value| value * value).sum::<f64>();
    let mut lambda = LM_INITIAL_LAMBDA;
    let mut up_factor = LM_INITIAL_UP_FACTOR;
    for _ in 0..maximum {
        let gradient_max = gradient.iter().fold(0.0_f64, |n, value| n.max(value.abs()));
        let damping: Parameters = std::array::from_fn(|i| {
            (a[i][i] * lambda).clamp(LM_MINIMUM_DIAGONAL, LM_MAXIMUM_DIAGONAL)
        });
        let mut damped = a;
        for i in 0..PARAMETERS {
            damped[i][i] = damping[i] + a[i][i];
        }
        let (singular, u, v) = svd(damped);
        let mut step = svd_solve(&singular, &u, &v, &gradient.map(|value| -value));
        let jacobian_cost_change = -0.5
            * step
                .iter()
                .enumerate()
                .map(|(i, value)| value * (gradient[i] - damping[i] * value))
                .sum::<f64>();
        // The default step criterion uses squared L2 norm before geodesic correction.
        let step_norm = step.iter().map(|value| value * value).sum::<f64>();
        let geo_x = std::array::from_fn(|i| x[i] + step[i] * LM_GEODESIC_STEP);
        let (_, geo_gradient, _) = normal_equations(from, to, &geo_x);
        let jtrvv = std::array::from_fn::<_, PARAMETERS, _>(|i| {
            (geo_gradient[i]
                + gradient[i] * (LM_GEODESIC_STEP - 1.0)
                + damping[i] * step[i] * LM_GEODESIC_STEP)
                / (LM_GEODESIC_STEP * LM_GEODESIC_STEP)
        });
        let geo_step = svd_solve(&singular, &u, &v, &jtrvv.map(|value| -value));
        let truncation_error = (geo_step.iter().map(|value| value * value).sum::<f64>()
            / step.iter().map(|value| value * value).sum::<f64>())
        .sqrt();
        if truncation_error < 1.0 {
            for i in 0..PARAMETERS {
                step[i] += geo_step[i] * LM_GEODESIC_SCALE;
            }
        }
        let probe = std::array::from_fn(|i| x[i] + step[i]);
        let (next_a, next_gradient, residual) = normal_equations(from, to, &probe);
        let next_energy = residual.iter().map(|value| value * value).sum::<f64>();
        // estimateAffine2D ignores the report and retains the last accepted parameters.
        if next_energy < 0.0 || next_energy.is_nan() {
            return;
        }
        let cost_change = energy - next_energy;
        if cost_change < 0.0 {
            lambda *= up_factor;
            up_factor *= LM_INITIAL_UP_FACTOR;
        } else {
            let quality = cost_change / jacobian_cost_change;
            lambda *= (1.0 / LM_DOWN_FACTOR).max(1.0 - (2.0 * quality - 1.0).powi(3));
            up_factor = LM_INITIAL_UP_FACTOR;
            *x = probe;
            a = next_a;
            gradient = next_gradient;
            energy = next_energy;
            if gradient_max < LM_TOLERANCE
                || step_norm < LM_TOLERANCE
                || cost_change / energy < LM_TOLERANCE
                || energy < 0.0
            {
                return;
            }
        }
        if lambda >= LM_MAXIMUM_LAMBDA {
            return;
        }
    }
}

// JacobiSVDImpl_<double>: six-by-six scalar path, U and V stored transposed.
fn svd(a: Matrix) -> (Parameters, Matrix, Matrix) {
    let mut u: Matrix = std::array::from_fn(|i| std::array::from_fn(|j| a[j][i]));
    let mut v = [[0.0; PARAMETERS]; PARAMETERS];
    let mut w: Parameters = std::array::from_fn(|i| u[i].iter().map(|value| value * value).sum());
    for (i, row) in v.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for _ in 0..PARAMETERS.max(SVD_MINIMUM_SWEEPS) {
        let mut changed = false;
        for i in 0..PARAMETERS - 1 {
            for j in i + 1..PARAMETERS {
                let a = w[i];
                let b = w[j];
                let mut p = u[i].iter().zip(u[j]).map(|(a, b)| a * b).sum::<f64>();
                if p.abs() <= SVD_EPSILON * (a * b).sqrt() {
                    continue;
                }
                p *= 2.0;
                let beta = a - b;
                let gamma = stable_hypot(p, beta);
                let (cosine, sine) = if beta < 0.0 {
                    let delta = (gamma - beta) * 0.5;
                    let sine = (delta / gamma).sqrt();
                    (p / (gamma * sine * 2.0), sine)
                } else {
                    let cosine = ((gamma + beta) / (gamma * 2.0)).sqrt();
                    (cosine, p / (gamma * cosine * 2.0))
                };
                for matrix in [&mut u, &mut v] {
                    let (before, after) = matrix.split_at_mut(j);
                    for (first, second) in before[i].iter_mut().zip(&mut after[0]) {
                        let t0 = cosine * *first + sine * *second;
                        let t1 = -sine * *first + cosine * *second;
                        *first = t0;
                        *second = t1;
                    }
                }
                w[i] = u[i].iter().map(|value| value * value).sum();
                w[j] = u[j].iter().map(|value| value * value).sum();
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (weight, row) in w.iter_mut().zip(&u) {
        *weight = row.iter().map(|value| value * value).sum::<f64>().sqrt();
    }
    for i in 0..PARAMETERS - 1 {
        let mut maximum = i;
        for k in i + 1..PARAMETERS {
            if w[maximum] < w[k] {
                maximum = k;
            }
        }
        if i != maximum {
            w.swap(i, maximum);
            u.swap(i, maximum);
            v.swap(i, maximum);
        }
    }
    let mut state = 0x1234_5678_u64;
    for i in 0..PARAMETERS {
        let mut norm = w[i];
        for _ in 0..SVD_ZERO_ATTEMPTS {
            if norm > f64::MIN_POSITIVE || norm.is_nan() {
                break;
            }
            for value in &mut u[i] {
                state = u64::from(state as u32) * RNG_COEFFICIENT + (state >> 32);
                let magnitude = 1.0 / PARAMETERS as f64;
                *value = if state as u32 & 256 != 0 {
                    magnitude
                } else {
                    -magnitude
                };
            }
            let (previous, current) = u.split_at_mut(i);
            let row = &mut current[0];
            for _ in 0..2 {
                for previous in previous.iter() {
                    let dot = row.iter().zip(previous).map(|(a, b)| a * b).sum::<f64>();
                    let mut absolute_sum = 0.0;
                    for (value, previous) in row.iter_mut().zip(previous) {
                        *value -= dot * previous;
                        absolute_sum += value.abs();
                    }
                    let scale = if absolute_sum > SVD_EPSILON * 100.0 {
                        1.0 / absolute_sum
                    } else {
                        0.0
                    };
                    for value in row.iter_mut() {
                        *value *= scale;
                    }
                }
            }
            norm = row.iter().map(|value| value * value).sum::<f64>().sqrt();
        }
        let scale = if norm > f64::MIN_POSITIVE {
            1.0 / norm
        } else {
            0.0
        };
        for value in &mut u[i] {
            *value *= scale;
        }
    }
    (w, u, v)
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

// SVBkSbImpl_<double>, transposed U/V, one right-hand side.
fn svd_solve(w: &Parameters, u: &Matrix, v: &Matrix, rhs: &Parameters) -> Parameters {
    let threshold = w.iter().sum::<f64>() * (f64::EPSILON * 2.0);
    let mut result = [0.0; PARAMETERS];
    for i in 0..PARAMETERS {
        if w[i].abs() <= threshold {
            continue;
        }
        let mut s = 0.0;
        for (coefficient, value) in u[i].iter().zip(rhs) {
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
