use sanim_core::math::chart::plot::{self, DensityOptions, Normalization};

#[test]
fn histograms_include_final_edge_and_normalize_unequal_width_bins() {
    let values = [0., 0.5, 1., 3.];
    let edges = [0., 1., 3.];
    assert_eq!(
        plot::histogram(&values, &edges, Normalization::Count).unwrap(),
        vec![2., 2.]
    );
    assert_eq!(
        plot::histogram(&values, &edges, Normalization::Probability).unwrap(),
        vec![0.5, 0.5]
    );
    let heights = plot::histogram(&values, &edges, Normalization::Density).unwrap();
    assert_eq!(heights, vec![0.5, 0.25]);
    assert_eq!(
        heights[0] * (edges[1] - edges[0]) + heights[1] * (edges[2] - edges[1]),
        1.
    );
    assert_eq!(
        plot::histogram(&[], &edges, Normalization::Density).unwrap(),
        vec![0., 0.]
    );
    assert!(
        plot::histogram(&[-1.], &edges, Normalization::Count)
            .unwrap_err()
            .contains("sample 1")
    );
    assert!(plot::histogram(&values, &[0., 1., 1.], Normalization::Count).is_err());
    assert!(plot::histogram(&[f64::NAN], &edges, Normalization::Count).is_err());
}

#[test]
fn monotone_curves_preserve_data_endpoints_and_segment_envelopes() {
    let data = [[0., 0.], [0.1, 1.], [3., 1.5], [7., 0.2]];
    let points = plot::curve(&data, "monotone", 16).unwrap();
    assert_eq!(points.len(), 49);
    assert_eq!(points[0], data[0]);
    assert_eq!(points[48], data[3]);
    for (segment, pair) in data.windows(2).enumerate() {
        for point in &points[segment * 16..=(segment + 1) * 16] {
            assert!(point[0] >= pair[0][0] && point[0] <= pair[1][0]);
            assert!(
                point[1] >= pair[0][1].min(pair[1][1]) && point[1] <= pair[0][1].max(pair[1][1])
            );
        }
    }
    assert!(plot::curve(&[[0., 0.], [0., 1.]], "monotone", 8).is_err());
    assert_eq!(
        plot::curve(&[[1., 0.], [0., 1.]], "linear", 8).unwrap(),
        vec![[1., 0.], [0., 1.]]
    );
    assert!(plot::curve(&data, "cubic", 8).is_err());
}

#[test]
fn gaussian_density_integrates_to_one_without_cropped_domain_renormalization() {
    let options = DensityOptions {
        domain: [-8., 8.],
        points: 1001,
        bandwidth: Some(0.5),
        max_work: 10_000,
    };
    let density = plot::density(&[-1., 0., 1.], options).unwrap();
    let integral = density
        .windows(2)
        .map(|pair| (pair[1][0] - pair[0][0]) * (pair[0][1] + pair[1][1]) / 2.)
        .sum::<f64>();
    assert!((integral - 1.).abs() < 1e-6);
    let cropped = plot::density(
        &[-1., 0., 1.],
        DensityOptions {
            domain: [-1., 1.],
            points: 101,
            ..options
        },
    )
    .unwrap();
    assert!((cropped[50][1] - density[500][1]).abs() < 1e-12);
    let normal = plot::normal_density(
        0.,
        2.,
        DensityOptions {
            domain: [-2., 2.],
            points: 3,
            ..options
        },
    )
    .unwrap();
    assert!((normal[1][1] - 0.19947114020071635).abs() < 1e-15);
    assert!(
        plot::density(
            &[1., 1.],
            DensityOptions {
                bandwidth: None,
                ..options
            }
        )
        .unwrap_err()
        .contains("constant")
    );
    assert!(
        plot::density(
            &[1., 2.],
            DensityOptions {
                max_work: 100,
                ..options
            }
        )
        .unwrap_err()
        .contains("kernel evaluations")
    );
    assert!(plot::normal_density(0., 0., options).is_err());
}

#[test]
fn ecdf_collapses_duplicates_and_extents_handle_constant_data() {
    assert_eq!(
        plot::ecdf(&[2., 1., 2., 3.]).unwrap(),
        vec![
            [1., 0.],
            [1., 0.25],
            [2., 0.25],
            [2., 0.75],
            [3., 0.75],
            [3., 1.]
        ]
    );
    assert!(plot::ecdf(&[]).unwrap().is_empty());
    assert!(
        plot::ecdf(&(0..plot::MAX_POINTS).map(|i| i as f64).collect::<Vec<_>>())
            .unwrap_err()
            .contains("vertices")
    );
    assert_eq!(
        plot::ecdf(&vec![1.; plot::MAX_POINTS]).unwrap(),
        vec![[1., 0.], [1., 1.]]
    );
    let range = plot::extent(&[2., 2.], 0., false).unwrap();
    assert_eq!(range, [1.9, 2.1]);
    assert_eq!(plot::extent(&[2., 3.], 0., true).unwrap(), [0., 3.]);
    assert!(plot::extent(&[], 0.05, false).is_err());
    assert!(plot::extent(&[1., f64::INFINITY], 0.05, false).is_err());
    assert!(plot::bin_edges([0., f64::EPSILON / 8.], 4096).is_ok());
    assert!(plot::bin_edges([1., 1. + f64::EPSILON], 20).is_err());
}
