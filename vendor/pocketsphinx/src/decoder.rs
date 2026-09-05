use crate::model::{AcousticModel, DENSITY_COUNT, FEATURE_DIM, PHONE_COUNT, SENONE_COUNT};
use crate::{PHONES, PhoneSegment};

const STATE_COUNT: usize = 3;
const LOG_BEAM: f32 = -46.051_7; // ln(1e-20)
const LANGUAGE_WEIGHT: f32 = 0.8;
const SILENCE_INDEX: usize = 36;
const IMPOSSIBLE: f32 = -1.0e30;

#[derive(Clone, Copy)]
struct Token {
    score: f32,
    history: Option<usize>,
}

impl Token {
    const DEAD: Self = Self {
        score: IMPOSSIBLE,
        history: None,
    };
}

struct History {
    phone: usize,
    end: i64,
    previous: Option<usize>,
}

pub(crate) fn decode(
    model: &AcousticModel,
    features: &[Vec<f32>],
) -> Result<Vec<PhoneSegment>, String> {
    if features.is_empty() {
        return Ok(Vec::new());
    }
    if features.iter().any(|feature| feature.len() != FEATURE_DIM) {
        return Err("invalid acoustic feature vector".to_owned());
    }

    let mut tokens = vec![Token::DEAD; PHONE_COUNT * STATE_COUNT];
    tokens[SILENCE_INDEX * STATE_COUNT] = Token {
        score: 0.0,
        history: None,
    };
    let mut histories = Vec::with_capacity(features.len() * PHONE_COUNT);
    let mut latest_exits = Vec::new();

    for (frame, feature) in features.iter().enumerate() {
        let emissions = acoustic_scores(model, feature);
        let mut next = vec![Token::DEAD; PHONE_COUNT * STATE_COUNT];
        let mut exits = vec![None; PHONE_COUNT];
        let mut best = IMPOSSIBLE;

        for (phone, exit_slot) in exits.iter_mut().enumerate() {
            let base = phone * STATE_COUNT;
            let old = &tokens[base..base + STATE_COUNT];
            if old.iter().all(|token| token.score <= IMPOSSIBLE / 2.0) {
                continue;
            }
            let state_ids = &model.states[base..base + STATE_COUNT];
            let emitted = [
                old[0].score + emissions[state_ids[0] as usize],
                old[1].score + emissions[state_ids[1] as usize],
                old[2].score + emissions[state_ids[2] as usize],
            ];
            let transition = model.transition_ids[phone] as usize * STATE_COUNT * 4;
            let matrix = &model.transitions[transition..transition + STATE_COUNT * 4];

            next[base] = Token {
                score: emitted[0] + matrix[0],
                history: old[0].history,
            };
            next[base + 1] = better(
                Token {
                    score: emitted[0] + matrix[1],
                    history: old[0].history,
                },
                Token {
                    score: emitted[1] + matrix[5],
                    history: old[1].history,
                },
            );
            next[base + 2] = better(
                better(
                    Token {
                        score: emitted[0] + matrix[2],
                        history: old[0].history,
                    },
                    Token {
                        score: emitted[1] + matrix[6],
                        history: old[1].history,
                    },
                ),
                Token {
                    score: emitted[2] + matrix[10],
                    history: old[2].history,
                },
            );
            let exit = better(
                Token {
                    score: emitted[1] + matrix[7],
                    history: old[1].history,
                },
                Token {
                    score: emitted[2] + matrix[11],
                    history: old[2].history,
                },
            );
            best = best
                .max(next[base].score)
                .max(next[base + 1].score)
                .max(next[base + 2].score)
                .max(exit.score);
            *exit_slot = Some(exit);
        }

        if best <= IMPOSSIBLE / 2.0 {
            return Err(format!("all-phone search failed at frame {frame}"));
        }
        let mut retained = [false; PHONE_COUNT];
        for (phone, exit) in exits.iter().copied().enumerate() {
            let base = phone * STATE_COUNT;
            let phone_best = next[base..base + STATE_COUNT]
                .iter()
                .map(|token| token.score)
                .chain(exit.map(|token| token.score))
                .fold(IMPOSSIBLE, f32::max);
            if phone_best < best + LOG_BEAM {
                next[base..base + STATE_COUNT].fill(Token::DEAD);
            } else {
                retained[phone] = true;
            }
        }

        latest_exits.clear();
        for (phone, exit) in exits.into_iter().enumerate() {
            let Some(exit) = exit else { continue };
            if !retained[phone] {
                continue;
            }
            let history_index = histories.len();
            histories.push(History {
                phone,
                end: frame as i64 + 1,
                previous: exit.history,
            });
            latest_exits.push((history_index, exit.score));

            let current_word = model.lm_words[phone];
            let previous_word = exit
                .history
                .map(|index| model.lm_words[histories[index].phone]);
            for target in 0..PHONE_COUNT {
                let target_word = model.lm_words[target];
                // Preserve the argument order used by this PocketSphinx
                // revision's all-phone search, which scores the history word
                // as the prediction and the target phone as oldest context.
                let language = if let Some(previous_word) = previous_word {
                    model.language_score(Some(target_word), current_word, previous_word)
                } else {
                    model.language_score(None, target_word, current_word)
                } * LANGUAGE_WEIGHT;
                let candidate = Token {
                    score: exit.score + language,
                    history: Some(history_index),
                };
                let target_index = target * STATE_COUNT;
                if candidate.score > next[target_index].score && candidate.score > best + LOG_BEAM {
                    next[target_index] = candidate;
                }
            }
        }
        // Renormalization preserves comparisons while preventing long clips
        // from exhausting f32 range.
        for token in &mut next {
            if token.score > IMPOSSIBLE / 2.0 {
                token.score -= best;
            }
        }
        tokens = next;
    }

    let best_history = latest_exits
        .iter()
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map(|entry| entry.0);
    let Some(mut history) = best_history else {
        return Ok(Vec::new());
    };
    let mut reversed = Vec::new();
    loop {
        let item = &histories[history];
        let start = item.previous.map(|index| histories[index].end).unwrap_or(0);
        reversed.push(PhoneSegment {
            start_centiseconds: start,
            end_centiseconds: item.end,
            phone: PHONES[item.phone],
        });
        let Some(previous) = item.previous else { break };
        history = previous;
    }
    reversed.reverse();
    Ok(reversed)
}

fn better(left: Token, right: Token) -> Token {
    if left.score >= right.score {
        left
    } else {
        right
    }
}

fn acoustic_scores(model: &AcousticModel, feature: &[f32]) -> Vec<f32> {
    let mut scores = vec![0.0; SENONE_COUNT];
    for (senone, output) in scores.iter_mut().enumerate() {
        let mut top = [(IMPOSSIBLE, 0_usize); 4];
        for density in 0..DENSITY_COUNT {
            let gaussian = senone * DENSITY_COUNT + density;
            let vector = gaussian * FEATURE_DIM;
            let distance = feature.iter().enumerate().fold(
                model.determinants[gaussian],
                |score, (dimension, value)| {
                    let difference = value - model.means[vector + dimension];
                    score - difference * difference * model.inverse_variances[vector + dimension]
                },
            );
            if distance > top[3].0 {
                top[3] = (distance, density);
                top.sort_unstable_by(|left, right| right.0.total_cmp(&left.0));
            }
        }
        let mixed = top.map(|(score, density)| {
            score + model.mixture_weights[senone * DENSITY_COUNT + density]
        });
        let maximum = mixed.into_iter().fold(IMPOSSIBLE, f32::max);
        *output = maximum
            + mixed
                .into_iter()
                .map(|score| (score - maximum).exp())
                .sum::<f32>()
                .ln();
    }
    scores
}
