use super::*;
use glam::DQuat;
use num_traits::ToPrimitive;

pub(super) fn position(point: Position, dimensions: [u32; 3]) -> Result<DVec3, String> {
    let mut result = [0.0; 3];
    for axis in 0..3 {
        result[axis] = crate::layout::coordinate(point[axis], dimensions[axis])?
            .to_f64()
            .filter(|value| value.is_finite())
            .ok_or("camera coordinates must be finite")?;
        if result[axis].abs() > MAX_COORDINATE {
            return Err(format!(
                "camera coordinates must resolve within ±{MAX_COORDINATE} cells"
            ));
        }
    }
    Ok(DVec3::from_array(result))
}

fn evaluate(path: &Compiled, parameter: f64) -> DVec3 {
    let segments = path.points.len() - usize::from(!path.closed);
    let parameter = parameter.clamp(0.0, segments as f64);
    let segment = (parameter.floor() as usize).min(segments - 1);
    let u = parameter - segment as f64;
    let n = path.points.len();
    let p1 = path.points[segment];
    let p2 = path.points[(segment + 1) % n];
    if path.interpolation == Interpolation::Linear {
        return p1.lerp(p2, u);
    }
    let p0 = if segment > 0 {
        path.points[segment - 1]
    } else if path.closed {
        path.points[n - 1]
    } else {
        2.0 * p1 - p2
    };
    let p3 = if segment + 2 < n {
        path.points[segment + 2]
    } else if path.closed {
        path.points[(segment + 2) % n]
    } else {
        2.0 * p2 - p1
    };
    let t0 = 0.0;
    let t1 = p0.distance(p1).sqrt();
    let t2 = t1 + p1.distance(p2).sqrt();
    let t3 = t2 + p2.distance(p3).sqrt();
    let t = t1 + (t2 - t1) * u;
    let a1 = p0.lerp(p1, (t - t0) / (t1 - t0));
    let a2 = p1.lerp(p2, (t - t1) / (t2 - t1));
    let a3 = p2.lerp(p3, (t - t2) / (t3 - t2));
    let b1 = a1.lerp(a2, (t - t0) / (t2 - t0));
    let b2 = a2.lerp(a3, (t - t1) / (t3 - t1));
    b1.lerp(b2, (t - t1) / (t2 - t1))
}

fn tangent(path: &Compiled, parameter: f64) -> DVec3 {
    const DIFFERENCE: f64 = 1e-4;
    let end = (path.points.len() - usize::from(!path.closed)) as f64;
    let a = if path.closed {
        (parameter - DIFFERENCE).rem_euclid(end)
    } else {
        (parameter - DIFFERENCE).max(0.0)
    };
    let b = if path.closed {
        (parameter + DIFFERENCE).rem_euclid(end)
    } else {
        (parameter + DIFFERENCE).min(end)
    };
    (evaluate(path, b) - evaluate(path, a))
        .try_normalize()
        .unwrap_or(DVec3::Z)
}

fn perpendicular(up: DVec3, forward: DVec3) -> DVec3 {
    (up - forward * up.dot(forward))
        .try_normalize()
        .unwrap_or_else(|| {
            let fallback = if forward.x.abs() < 0.9 {
                DVec3::X
            } else {
                DVec3::Y
            };
            (fallback - forward * fallback.dot(forward)).normalize()
        })
}

pub(super) fn compile(definition: &Definition, dimensions: [u32; 3]) -> Result<Compiled, String> {
    let mut points = Vec::with_capacity(definition.points.len());
    for point in definition.points.iter() {
        let point = position(*point, dimensions)?;
        if points
            .last()
            .is_none_or(|last: &DVec3| last.distance_squared(point) > f64::EPSILON)
        {
            points.push(point);
        }
    }
    if definition.closed
        && points.len() > 1
        && points[0].distance_squared(*points.last().unwrap()) <= f64::EPSILON
    {
        points.pop();
    }
    if points.len() < 2 {
        return Err("Curve resolves to no movement; provide distinct points".into());
    }
    let mut path = Compiled {
        points,
        entries: Vec::new(),
        interpolation: definition.interpolation,
        closed: definition.closed,
    };
    let count = (path.points.len() - usize::from(!path.closed)) * SAMPLES_PER_SEGMENT;
    path.entries = Vec::with_capacity(count + 1);
    let mut previous = evaluate(&path, 0.0);
    let mut forward = tangent(&path, 0.0);
    let mut up = perpendicular(DVec3::Y, forward);
    let mut distance = 0.0;
    for index in 0..=count {
        let parameter = index as f64 / SAMPLES_PER_SEGMENT as f64;
        let point = evaluate(&path, parameter);
        let next_forward = tangent(&path, parameter);
        if forward.dot(next_forward) > -1.0 + 1e-9 {
            up = DQuat::from_rotation_arc(forward, next_forward) * up;
        }
        up = perpendicular(up, next_forward);
        distance += previous.distance(point);
        path.entries.push(Entry {
            distance,
            parameter,
            up,
        });
        previous = point;
        forward = next_forward;
    }
    if path.closed {
        let first = path.entries.first().unwrap().up;
        let last = path.entries.last().unwrap().up;
        let angle = forward.dot(last.cross(first)).atan2(last.dot(first));
        for index in 0..path.entries.len() {
            let entry = &path.entries[index];
            let correction = DQuat::from_axis_angle(
                tangent(&path, entry.parameter),
                angle * entry.distance / distance,
            );
            let up = correction * entry.up;
            path.entries[index].up = up;
        }
    }
    Ok(path)
}

fn at_distance(path: &Compiled, distance: f64) -> (f64, DVec3) {
    let end = path.entries.last().unwrap().distance;
    let distance = if path.closed {
        distance.rem_euclid(end)
    } else {
        distance.clamp(0.0, end)
    };
    let high = path
        .entries
        .partition_point(|entry| entry.distance < distance)
        .clamp(1, path.entries.len() - 1);
    let a = &path.entries[high - 1];
    let b = &path.entries[high];
    let u = if b.distance > a.distance {
        (distance - a.distance) / (b.distance - a.distance)
    } else {
        0.0
    };
    (
        a.parameter + (b.parameter - a.parameter) * u,
        a.up.lerp(b.up, u),
    )
}

pub(super) fn sample(path: &Compiled, progress: f64, look_ahead: f64) -> Sample {
    let length = path.entries.last().unwrap().distance;
    let progress = if path.closed {
        progress.rem_euclid(1.0)
    } else {
        progress.clamp(0.0, 1.0)
    };
    let (parameter, up) = at_distance(path, progress * length);
    let position = evaluate(path, parameter);
    let forward = tangent(path, parameter);
    let distance = progress * length;
    let target = if look_ahead > 0.0 {
        let ahead = evaluate(path, at_distance(path, distance + look_ahead).0);
        if ahead.distance_squared(position) > f64::EPSILON {
            ahead
        } else {
            position + forward
        }
    } else {
        position + forward
    };
    Sample {
        position: position.to_array(),
        target: target.to_array(),
        up: perpendicular(up, (target - position).normalize()).to_array(),
        distance,
        length,
    }
}
