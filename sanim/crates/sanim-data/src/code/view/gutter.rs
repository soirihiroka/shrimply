//! Shared gutter metrics and native scope controls for hard and soft code rows.
use super::*;

pub(super) struct Gutter {
    pub number_width: f64,
    pub left: f64,
    scope_x: f64,
    space: f64,
}
impl Gutter {
    pub fn new(options: &Options, lines: usize, space: f64) -> Result<Self, String> {
        let number_width = if options.line_numbers {
            run(
                &options.font,
                options.font_size,
                &"0".repeat(lines.to_string().len()),
                &Style::default(),
            )?
            .advance
        } else {
            0.
        };
        let number_column = if options.line_numbers {
            number_width + space
        } else {
            0.
        };
        Ok(Self {
            number_width,
            left: number_column
                + if options.scope_indicators { space } else { 0. }
                + if options.line_numbers || options.scope_indicators {
                    space
                } else {
                    0.
                },
            scope_x: number_column + space / 2.,
            space,
        })
    }
    pub fn indicators(
        &self,
        id: &str,
        document: &Document,
        options: &Options,
        folded: &imbl::OrdSet<Arc<str>>,
        revision: [u8; 32],
    ) -> Result<BTreeMap<usize, Token>, String> {
        if !options.scope_indicators {
            return Ok(BTreeMap::new());
        }
        // A scope needs complete body lines to collapse. Multiple nested scopes
        // starting on one header share its outermost control, like an editor.
        let mut scopes = BTreeMap::<usize, &sanim_core::code::Scope>::new();
        for scope in document.scopes.iter() {
            if scope.hidden_end_line(&document.text)? <= scope.start_line {
                continue;
            }
            scopes
                .entry(scope.start_line)
                .and_modify(|previous| {
                    if (scope.depth, std::cmp::Reverse(scope.end_line))
                        < (previous.depth, std::cmp::Reverse(previous.end_line))
                    {
                        *previous = scope;
                    }
                })
                .or_insert(scope);
            if scopes.len() > MAX_CODE_TOKENS {
                return Err(format!(
                    "CodeView exceeds {MAX_CODE_TOKENS} scope controls; hide scope_indicators or show a shorter file"
                ));
            }
        }
        let half = self.space * 0.35;
        let geometry = Arc::new(Geometry::Polyline {
            points: Arc::new(vec![
                Vec2 {
                    values: [-half, -half * 2.],
                    unit: Unit::Length,
                },
                Vec2 {
                    values: [half, 0.],
                    unit: Unit::Length,
                },
                Vec2 {
                    values: [-half, half * 2.],
                    unit: Unit::Length,
                },
            ]),
            width: options.font_size / Fraction::from(16),
        });
        let mut output = BTreeMap::new();
        for (line, scope) in scopes {
            let key: Arc<str> = format!("scope-indicator:{}", scope.id).into();
            let position = local_position(
                &geometry,
                [self.scope_x, -math::to_f64(options.font_size)? / 3.],
            )?;
            let mut glyph = element(
                format!("{id}/@scope/{}", scope.id),
                geometry.clone(),
                position,
                options.theme.line_number.clone(),
                1.into(),
            );
            glyph.rotation = if folded.contains(&scope.id) {
                0.into()
            } else {
                90.into()
            };
            output.insert(
                line,
                Token {
                    revision,
                    key,
                    style: Arc::new(Style {
                        fill: Some(options.theme.line_number.clone()),
                        ..Default::default()
                    }),
                    element: Arc::new(glyph),
                    position: Vec2 {
                        values: position,
                        unit: Unit::Length,
                    },
                    line,
                    column: 0,
                },
            );
        }
        Ok(output)
    }
}
