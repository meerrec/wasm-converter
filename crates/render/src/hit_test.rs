//! Попадание точки в команды DisplayList (ADR-0003).
//!
//! Чистая геометрия над кадром: верхняя команда под точкой выигрывает, а
//! `PushClip`/`PopClip` ограничивают область поиска. `PushTransform` не
//! учитывается — xlsx его не эмитит, а следить за аффинным преобразованием
//! здесь нечего, пока второй потребитель (DOCX) не появился.

use crate::display_list::{DisplayList, DrawCommand, StringRef};
use crate::geometry::{Point, Rect};

/// Команда, в которую попала точка.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitTarget {
    Rect(Rect),
    Image {
        rect: Rect,
        bitmap_id: u32,
    },
    /// Текст. DisplayList v3 не несёт ширину текста, поэтому цель — якорь в
    /// точке `(x, y)` с нулевым прямоугольником.
    ///
    /// TODO (Фаза 4): настоящие границы текста, когда кадр начнёт нести метрики.
    Text {
        x: f32,
        y: f32,
        text: StringRef,
    },
}

impl HitTarget {
    /// Прямоугольник цели в физических пикселях canvas.
    #[must_use]
    pub fn rect(&self) -> Rect {
        match *self {
            Self::Rect(rect) => rect,
            Self::Image { rect, .. } => rect,
            Self::Text { x, y, .. } => Rect::new(x, y, 0.0, 0.0),
        }
    }
}

/// Прямоугольник цели — псевдоним [`HitTarget::rect`] из ROADMAP.
#[must_use]
pub fn rect_for(target: &HitTarget) -> Rect {
    target.rect()
}

/// Самая верхняя команда под точкой, с учётом клипов.
///
/// Команды обходятся с конца: верхняя нарисована позже и побеждает. Наборы
/// активных клипов расставлены прямым проходом заранее — прямоугольник
/// отсечения лежит в `PushClip`, то есть раньше содержимого, и при обратном
/// обходе узнать его из `PopClip` уже неоткуда.
#[must_use]
pub fn hit_test(dl: &DisplayList, px: f32, py: f32) -> Option<HitTarget> {
    let point = Point::new(px, py);
    let clips = clip_sets(dl);
    let inside_clips = |clips: &[Rect]| clips.iter().all(|clip| clip.contains(point));

    for index in (0..dl.len()).rev() {
        let Some(command) = dl.cmd(index) else {
            continue;
        };
        let active = &clips[index];
        match command {
            DrawCommand::Rect { x, y, w, h, .. } => {
                let rect = Rect::new(*x, *y, *w, *h);
                if rect.contains(point) && inside_clips(active) {
                    return Some(HitTarget::Rect(rect));
                }
            }
            DrawCommand::Image {
                x,
                y,
                w,
                h,
                bitmap_id,
            } => {
                let rect = Rect::new(*x, *y, *w, *h);
                if rect.contains(point) && inside_clips(active) {
                    return Some(HitTarget::Image {
                        rect,
                        bitmap_id: *bitmap_id,
                    });
                }
            }
            DrawCommand::Text { x, y, text, .. } => {
                if Point::new(*x, *y) == point && inside_clips(active) {
                    return Some(HitTarget::Text {
                        x: *x,
                        y: *y,
                        text: *text,
                    });
                }
            }
            DrawCommand::Clear
            | DrawCommand::Line { .. }
            | DrawCommand::PushClip { .. }
            | DrawCommand::PopClip
            | DrawCommand::PushTransform { .. }
            | DrawCommand::PopTransform => {}
        }
    }
    None
}

/// Для каждой команды — набор активных прямоугольников отсечения.
fn clip_sets(dl: &DisplayList) -> Vec<Vec<Rect>> {
    let mut sets = Vec::with_capacity(dl.len());
    let mut stack: Vec<Rect> = Vec::new();
    for index in 0..dl.len() {
        let Some(command) = dl.cmd(index) else {
            sets.push(stack.clone());
            continue;
        };
        match command {
            DrawCommand::PushClip { x, y, w, h } => {
                sets.push(stack.clone());
                stack.push(Rect::new(*x, *y, *w, *h));
                continue;
            }
            DrawCommand::PopClip => {
                stack.pop();
            }
            _ => {}
        }
        sets.push(stack.clone());
    }
    sets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_list::{Color, DisplayList};

    fn rect(x: f32, y: f32, w: f32, h: f32) -> DrawCommand {
        DrawCommand::Rect {
            x,
            y,
            w,
            h,
            fill: Color::BLACK,
            stroke: Color::TRANSPARENT,
            stroke_w: 0.0,
            radius: [0.0; 4],
        }
    }

    #[test]
    fn topmost_rect_wins() {
        let mut dl = DisplayList::new();
        dl.push(rect(0.0, 0.0, 100.0, 100.0));
        dl.push(rect(20.0, 20.0, 100.0, 100.0));

        let hit = hit_test(&dl, 50.0, 50.0);
        assert_eq!(
            hit,
            Some(HitTarget::Rect(Rect::new(20.0, 20.0, 100.0, 100.0)))
        );
    }

    #[test]
    fn nested_clips_intersect() {
        let mut dl = DisplayList::new();
        dl.push(DrawCommand::PushClip {
            x: 0.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
        });
        dl.push(DrawCommand::PushClip {
            x: 10.0,
            y: 10.0,
            w: 20.0,
            h: 20.0,
        });
        dl.push(rect(0.0, 0.0, 100.0, 100.0));
        dl.push(DrawCommand::PopClip);
        dl.push(DrawCommand::PopClip);

        // В пересечении клипов — цель есть, вне — нет.
        assert_eq!(
            hit_test(&dl, 15.0, 15.0),
            Some(HitTarget::Rect(Rect::new(0.0, 0.0, 100.0, 100.0)))
        );
        assert!(hit_test(&dl, 5.0, 15.0).is_none());
    }

    #[test]
    fn clip_excludes_commands_outside() {
        let mut dl = DisplayList::new();
        dl.push(DrawCommand::PushClip {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        });
        dl.push(rect(0.0, 0.0, 100.0, 100.0));
        dl.push(DrawCommand::PopClip);

        assert!(hit_test(&dl, 20.0, 20.0).is_none());
        assert_eq!(
            hit_test(&dl, 5.0, 5.0),
            Some(HitTarget::Rect(Rect::new(0.0, 0.0, 100.0, 100.0)))
        );
    }

    #[test]
    fn pop_clip_restores_visibility() {
        let mut dl = DisplayList::new();
        dl.push(DrawCommand::PushClip {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        });
        dl.push(rect(0.0, 0.0, 100.0, 100.0));
        dl.push(DrawCommand::PopClip);
        dl.push(rect(0.0, 0.0, 200.0, 200.0));

        assert_eq!(
            hit_test(&dl, 150.0, 150.0),
            Some(HitTarget::Rect(Rect::new(0.0, 0.0, 200.0, 200.0)))
        );
    }

    #[test]
    fn image_and_text_targets() {
        let mut dl = DisplayList::new();
        dl.push(DrawCommand::Image {
            x: 10.0,
            y: 10.0,
            w: 40.0,
            h: 40.0,
            bitmap_id: 7,
        });
        let text = dl.intern("hello");
        dl.push(DrawCommand::Text {
            x: 100.0,
            y: 100.0,
            text,
            font: text,
            size: 12.0,
            color: Color::BLACK,
            align: crate::display_list::TextAlign::Left,
            baseline: crate::display_list::TextBaseline::Alphabetic,
            bold: false,
            italic: false,
            underline: false,
        });

        assert_eq!(
            hit_test(&dl, 30.0, 30.0),
            Some(HitTarget::Image {
                rect: Rect::new(10.0, 10.0, 40.0, 40.0),
                bitmap_id: 7,
            })
        );
        assert_eq!(
            hit_test(&dl, 100.0, 100.0),
            Some(HitTarget::Text {
                x: 100.0,
                y: 100.0,
                text,
            })
        );
        assert_eq!(
            rect_for(&HitTarget::Image {
                rect: Rect::new(1.0, 2.0, 3.0, 4.0),
                bitmap_id: 0,
            }),
            Rect::new(1.0, 2.0, 3.0, 4.0)
        );
    }

    #[test]
    fn no_target_outside() {
        let mut dl = DisplayList::new();
        dl.push(rect(0.0, 0.0, 10.0, 10.0));
        assert!(hit_test(&dl, 20.0, 20.0).is_none());
        assert!(hit_test(&DisplayList::new(), 0.0, 0.0).is_none());
    }
}
