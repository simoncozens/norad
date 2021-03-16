use kurbo::{Affine, BezPath, Line, ParamCurve, PathEl, Point, Rect, Shape};
use norad::glyph::{Component, Contour, ContourPoint, Glyph, PointType};
use norad::{GlifVersion, GlyphBuilder, GlyphName, Layer, OutlineBuilder};

// TODO:
// - Write `set_(left|right)_margin` plus `move`
// - Write deslanter for italic sidebearings
// - Write new_layer
// - Make spacing polygons BezPaths for free `area` fn?

fn main() {
    for arg in std::env::args().skip(1) {
        let mut ufo = match norad::Ufo::load(&arg) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Loading UFO failed: {}", e);
                std::process::exit(1);
            }
        };

        let units_per_em: f64 =
            ufo.font_info.as_ref().map_or(1000.0, |info| match info.units_per_em {
                Some(a) => a.get(),
                None => 1000.0,
            });
        let angle: f64 = ufo.font_info.as_ref().map_or(0.0, |info| match info.italic_angle {
            Some(a) => -a.get(),
            None => 0.0,
        });
        let xheight: f64 = ufo.font_info.as_ref().map_or(0.0, |info| match info.x_height {
            Some(a) => a.get(),
            None => 0.0,
        });
        let param_area: f64 = 400.0;
        let param_depth: f64 = 15.0;
        let param_overshoot: f64 = 0.0;
        let overshoot = xheight * param_overshoot / 100.0;
        let param_sample_frequency: usize = 5;

        let default_layer = ufo.get_default_layer().unwrap();
        let mut background_glyphs: Vec<Glyph> = Vec::new();

        for glyph in default_layer.iter_contents() {
            let (factor, glyph_reference) = config_for_glyph(&glyph, &default_layer);

            let paths = match path_for_glyph(&glyph, &default_layer) {
                Ok(maybe_path) => match maybe_path {
                    Some(path) => path,
                    None => continue,
                },
                Err(e) => {
                    println!("Error while drawing {}: {:?}", glyph.name, e);
                    continue;
                }
            };
            let bounds = paths.bounding_box();
            let paths_reference = match path_for_glyph(&glyph_reference, &default_layer) {
                Ok(maybe_path) => match maybe_path {
                    Some(path) => path,
                    None => continue,
                },
                Err(e) => {
                    println!("Error while drawing {}: {:?}", glyph_reference.name, e);
                    continue;
                }
            };
            let bounds_reference = paths_reference.bounding_box();
            let bounds_reference_lower = (bounds_reference.min_y() - overshoot).round();
            let bounds_reference_upper = (bounds_reference.max_y() + overshoot).round();

            let (new_left, new_right) = calculate_spacing(
                paths,
                bounds,
                (bounds_reference_lower, bounds_reference_upper),
                angle,
                xheight,
                param_sample_frequency,
                param_depth,
                glyph.name.clone(),
                &mut background_glyphs,
                factor,
                param_area,
                units_per_em,
            );

            println!("{}: {:?}, {:?}", glyph.name, new_left, new_right);
        }

        // Write out background layer.
        // TODO: write Ufo::new_layer method.
        let mut background_layer = norad::LayerInfo {
            name: "public.background".into(),
            path: std::path::PathBuf::from("glyphs.background"),
            layer: Layer::default(),
        };
        for glyph in background_glyphs {
            background_layer.layer.insert_glyph(glyph)
        }
        ufo.layers.push(background_layer);

        ufo.meta.creator = "org.linebender.norad".into();
        let output_path = std::path::PathBuf::from(&arg);
        ufo.save(std::path::PathBuf::from("/tmp").join(output_path.file_name().unwrap())).unwrap();
    }
}

/// Returns the factor and reference glyph to be used for a glyph.
///
/// A rough port of HTLetterspacer's default configuration, as Glyphs.app provides richer metadata
/// for glyph names and Unicode codepoints.
fn config_for_glyph<'a>(glyph: &'a Glyph, glyphset: &'a Layer) -> (f64, &'a Glyph) {
    use unic_ucd_category::GeneralCategory::*;

    let glyph_ref_or_self =
        |name: &str| glyphset.get_glyph(name).map(|g| g.as_ref()).unwrap_or(glyph);

    match determine_unicode(glyph, glyphset) {
        Some(u) => {
            let category = unic_ucd_category::GeneralCategory::of(u);
            match category {
                UppercaseLetter => (1.25, glyph_ref_or_self("H")),
                LowercaseLetter => {
                    if glyph.name.contains(".sc") {
                        (1.1, glyph_ref_or_self("h.sc"))
                    } else if glyph.name.contains(".sups") {
                        (0.7, glyph_ref_or_self("m.sups"))
                    } else {
                        (1.0, glyph_ref_or_self("x"))
                    }
                }
                DecimalNumber => {
                    if glyph.name.contains(".osf") {
                        (1.2, glyph_ref_or_self("zero.osf"))
                    } else {
                        (1.2, glyph_ref_or_self("one"))
                    }
                }
                OtherNumber => {
                    // Skips special treatment for fractions because bare Unicode is missing info on that.
                    if glyph.name.contains(".dnom")
                        || glyph.name.contains(".numr")
                        || glyph.name.contains(".inferior")
                        || glyph.name.contains("superior")
                    {
                        (0.8, glyph)
                    } else {
                        (1.0, glyph)
                    }
                }
                OpenPunctuation | ClosePunctuation | InitialPunctuation | FinalPunctuation => {
                    (1.2, glyph)
                }
                OtherPunctuation => {
                    if u == '/' {
                        (1.0, glyph)
                    } else {
                        (1.4, glyph)
                    }
                }
                CurrencySymbol => (1.6, glyph),
                MathSymbol | OtherSymbol => (1.5, glyph),
                _ => (1.0, glyph),
            }
        }
        _ => {
            if &*glyph.name == "IJ" {
                (1.25, glyph_ref_or_self("H"))
            } else {
                (1.0, glyph)
            }
        }
    }
}

fn calculate_spacing(
    paths: BezPath,
    bounds: Rect,
    (bounds_reference_lower, bounds_reference_upper): (f64, f64),
    angle: f64,
    xheight: f64,
    param_sample_frequency: usize,
    param_depth: f64,
    glyph_name: impl Into<GlyphName>,
    background_glyphs: &mut Vec<Glyph>,
    factor: f64,
    param_area: f64,
    units_per_em: f64,
) -> (Option<f64>, Option<f64>) {
    if paths.is_empty() {
        return (None, None);
    }

    let (left, extreme_left_full, extreme_left, right, extreme_right_full, extreme_right) =
        spacing_polygons(
            &paths,
            &bounds,
            (bounds_reference_lower, bounds_reference_upper),
            angle,
            xheight,
            param_sample_frequency,
            param_depth,
        );

    let background_glyph = draw_glyph_outer_outline_into_glyph(glyph_name, (&left, &right));
    background_glyphs.push(background_glyph);

    // Difference between extreme points full and in zone.
    let distance_left = (extreme_left.x - extreme_left_full.x).ceil();
    let distance_right = (extreme_right_full.x - extreme_right.x).ceil();

    let new_left = (-distance_left
        + calculate_sidebearing_value(
            factor,
            (bounds_reference_lower, bounds_reference_upper),
            param_area,
            &left,
            units_per_em,
            xheight,
        ))
    .ceil();
    let new_right = (-distance_right
        + calculate_sidebearing_value(
            factor,
            (bounds_reference_lower, bounds_reference_upper),
            param_area,
            &right,
            units_per_em,
            xheight,
        ))
    .ceil();

    (Some(new_left), Some(new_right))
}

fn determine_unicode(glyph: &Glyph, glyphset: &Layer) -> Option<char> {
    if let Some(codepoints) = &glyph.codepoints {
        if let Some(codepoint) = codepoints.first() {
            return Some(*codepoint);
        }
    }

    let base_name = glyph.name.split(".").nth(0).unwrap();
    if let Some(base_glyph) = glyphset.get_glyph(base_name) {
        if let Some(codepoints) = &base_glyph.codepoints {
            if let Some(codepoint) = codepoints.first() {
                return Some(*codepoint);
            }
        }
    }

    None
}

fn calculate_sidebearing_value(
    factor: f64,
    (bounds_reference_lower, bounds_reference_upper): (f64, f64),
    param_area: f64,
    polygon: &Vec<Point>,
    units_per_em: f64,
    xheight: f64,
) -> f64 {
    let amplitude_y = bounds_reference_upper - bounds_reference_lower;
    let area_upm = param_area * (units_per_em / 1000.0).powi(2);
    let white_area = area_upm * factor * 100.0;
    let prop_area = (amplitude_y * white_area) / xheight;
    let valor = prop_area - area(&polygon);
    valor / amplitude_y
}

fn area(points: &Vec<Point>) -> f64 {
    // https://mathopenref.com/coordpolygonarea2.html
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .fold(0.0, |sum, (prev, next)| sum + (prev.x * next.y - next.x * prev.y))
        .abs()
        / 2.0
}

fn spacing_polygons(
    paths: &BezPath,
    bounds: &Rect,
    (bounds_reference_lower, bounds_reference_upper): (f64, f64),
    angle: f64,
    xheight: f64,
    scan_frequency: usize,
    depth_cut: f64,
) -> (Vec<Point>, Point, Point, Vec<Point>, Point, Point) {
    // For deskewing angled glyphs. Makes subsequent processing easier.
    let skew_offset = xheight / 2.0;
    let tan_angle = angle.to_radians().tan();

    // First pass: Collect the outer intersections of a horizontal line with the glyph on both sides, going bottom
    // to top. The spacing polygon is vertically limited to lower_bound_reference..=upper_bound_reference,
    // but we need to collect the extreme points on both sides for the full stretch for spacing later.

    // A glyph can over- or undershoot its reference bounds. Measure the tallest stretch.
    let bounds_sampling_lower = bounds.min_y().round().min(bounds_reference_lower) as isize;
    let bounds_sampling_upper = bounds.max_y().round().max(bounds_reference_upper) as isize;

    let mut left = Vec::new();
    let left_bounds = bounds.min_x();
    let mut extreme_left_full: Option<Point> = None;
    let mut extreme_left: Option<Point> = None;
    let mut right = Vec::new();
    let right_bounds = bounds.max_x();
    let mut extreme_right_full: Option<Point> = None;
    let mut extreme_right: Option<Point> = None;
    for y in
        (bounds_sampling_lower..=bounds_sampling_upper).step_by(scan_frequency).map(|v| v as f64)
    {
        let line = Line::new((left_bounds, y), (right_bounds, y));
        let in_reference_zone = bounds_reference_lower <= y && y <= bounds_reference_upper;

        let mut hits = intersections_for_line(paths, line);
        if hits.is_empty() {
            if in_reference_zone {
                // Treat no hits as hits deep off the other side.
                left.push(Point::new(f64::INFINITY, y));
                right.push(Point::new(-f64::INFINITY, y));
            }
        } else {
            hits.sort_by_key(|k| k.x.round() as i32);
            let mut first = hits.first().unwrap().clone(); // XXX: don't clone but own?
            let mut last = hits.last().unwrap().clone();
            if angle != 0.0 {
                first = Point::new(first.x - (y - skew_offset) * tan_angle, first.y);
                last = Point::new(last.x - (y - skew_offset) * tan_angle, last.y);
            }
            if in_reference_zone {
                left.push(first);
                right.push(last);

                extreme_left = extreme_left
                    .map(|l| if l.x < first.x { l } else { first.clone() })
                    .or(Some(first.clone()));
                extreme_right = extreme_right
                    .map(|r| if r.x > last.x { r } else { last.clone() })
                    .or(Some(last.clone()));
            }

            extreme_left_full = extreme_left_full
                .map(|l| if l.x < first.x { l } else { first.clone() })
                .or(Some(first.clone()));
            extreme_right_full = extreme_right_full
                .map(|r| if r.x > last.x { r } else { last.clone() })
                .or(Some(last.clone()));
        }
    }

    let extreme_left_full = extreme_left_full.unwrap();
    let extreme_left = extreme_left.unwrap();
    let extreme_right_full = extreme_right_full.unwrap();
    let extreme_right = extreme_right.unwrap();

    // Second pass: Cap the margin samples to a maximum depth from the outermost point in to get our depth cut-in.
    let depth = xheight * depth_cut / 100.0;
    let max_depth = extreme_left.x + depth;
    let min_depth = extreme_right.x - depth;
    left.iter_mut().for_each(|s| s.x = s.x.min(max_depth));
    right.iter_mut().for_each(|s| s.x = s.x.max(min_depth));

    // Third pass: Close open counterforms at 45 degrees.
    let dx_max = scan_frequency as f64;

    for i in 0..left.len() - 1 {
        if left[i + 1].x - left[i].x > dx_max {
            left[i + 1].x = left[i].x + dx_max;
        }
        if right[i + 1].x - right[i].x < -dx_max {
            right[i + 1].x = right[i].x - dx_max;
        }
    }
    for i in (0..left.len() - 1).rev() {
        if left[i].x - left[i + 1].x > dx_max {
            left[i].x = left[i + 1].x + dx_max;
        }
        if right[i].x - right[i + 1].x < -dx_max {
            right[i].x = right[i + 1].x - dx_max;
        }
    }

    left.insert(0, Point { x: extreme_left.x, y: bounds_reference_lower as f64 });
    left.push(Point { x: extreme_left.x, y: bounds_reference_upper as f64 });
    right.insert(0, Point { x: extreme_right.x, y: bounds_reference_lower as f64 });
    right.push(Point { x: extreme_right.x, y: bounds_reference_upper as f64 });

    (left, extreme_left_full, extreme_left, right, extreme_right_full, extreme_right)
}

fn intersections_for_line(paths: &BezPath, line: Line) -> Vec<Point> {
    paths
        .segments()
        .flat_map(|s| s.intersect_line(line).into_iter().map(move |h| s.eval(h.segment_t).round()))
        .collect()
}

fn draw_glyph_outer_outline_into_glyph(
    glyph_name: impl Into<GlyphName>,
    outlines: (&Vec<Point>, &Vec<Point>),
) -> Glyph {
    let mut builder = GlyphBuilder::new(glyph_name, GlifVersion::V2);
    let mut outline_builder = OutlineBuilder::new();
    outline_builder.begin_path(None).unwrap();
    for left in outlines.0 {
        outline_builder
            .add_point(
                (left.x.round() as f32, left.y.round() as f32),
                PointType::Line,
                false,
                None,
                None,
            )
            .unwrap();
    }
    outline_builder.end_path().unwrap();
    outline_builder.begin_path(None).unwrap();
    for right in outlines.1 {
        outline_builder
            .add_point(
                (right.x.round() as f32, right.y.round() as f32),
                PointType::Line,
                false,
                None,
                None,
            )
            .unwrap();
    }
    outline_builder.end_path().unwrap();
    let (outline, identifiers) = outline_builder.finish().unwrap();
    builder.outline(outline, identifiers).unwrap();
    builder.finish().unwrap()
}

/// Returns a Vec of decomposed components of a composite. Ignores incoming identifiers and libs
/// and dangling components; contours are in no particular order.
fn decomposed_components(glyph: &Glyph, glyphset: &Layer) -> Vec<Contour> {
    let mut contours = Vec::new();

    if let Some(outline) = &glyph.outline {
        let mut stack: Vec<(&Component, Affine)> = Vec::new();

        for component in &outline.components {
            stack.push((component, component.transform.into()));

            while let Some((component, transform)) = stack.pop() {
                let new_outline = match glyphset.get_glyph(&component.base) {
                    Some(g) => match &g.outline {
                        Some(o) => o,
                        None => continue,
                    },
                    None => continue,
                };

                for contour in &new_outline.contours {
                    let mut decomposed_contour = Contour::default();
                    for point in &contour.points {
                        let new_point = transform * Point::new(point.x as f64, point.y as f64);
                        decomposed_contour.points.push(ContourPoint::new(
                            new_point.x as f32,
                            new_point.y as f32,
                            point.typ.clone(),
                            point.smooth,
                            point.name.clone(),
                            None,
                            None,
                        ))
                    }
                    contours.push(decomposed_contour);
                }

                for new_component in new_outline.components.iter().rev() {
                    let new_transform: Affine = new_component.transform.into();
                    stack.push((new_component, transform * new_transform));
                }
            }
        }
    }

    contours
}

fn path_for_glyph(glyph: &Glyph, glyphset: &Layer) -> Result<Option<BezPath>, ContourDrawingError> {
    if let Some(outline) = glyph.outline.as_ref() {
        let mut path = BezPath::new();
        for contour in outline.contours.iter().chain(decomposed_components(glyph, glyphset).iter())
        {
            for element in contour_segments(contour)? {
                path.push(element);
            }
        }
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

#[derive(Debug)]
enum ContourDrawingError {
    IllegalPointCount(PointType, usize),
    IllegalMove,
    TrailingOffCurves,
}

fn contour_segments(contour: &Contour) -> Result<Vec<PathEl>, ContourDrawingError> {
    let mut points: Vec<&ContourPoint> = contour.points.iter().collect();
    let mut segments = Vec::new();

    // If we have 2 points or more and aren't an open contour (first point is a move),
    // locate the first on-curve point and rotate the point list so that it _ends_ with
    // that point. The first point could be a curve with its off-curves at the end; moving
    // the point makes always makes all associated off-curves reachable in a single pass
    // without wrapping around.
    let mut start: Option<&ContourPoint> = None;
    let mut closed = true;
    let implied_oncurve: ContourPoint;

    match points.len() {
        0 => return Ok(segments),
        1 => {
            let point = points[0];
            segments.push(PathEl::MoveTo(Point::new(point.x as f64, point.y as f64)));
            return Ok(segments);
        }
        _ => {
            if points[0].typ == PointType::Move {
                closed = false;
                start = Some(points.remove(0));
            } else {
                if let Some(first_oncurve) =
                    points.iter().position(|e| e.typ != PointType::OffCurve)
                {
                    points.rotate_left(first_oncurve + 1);
                    start = Some(points.last().unwrap());
                } else {
                    // We are an all-offcurve quad blob. Expand implied oncurves and moveto on last one.
                    // Do all processing here and return?
                    let first = points.first().unwrap();
                    let last = points.last().unwrap();
                    implied_oncurve = ContourPoint::new(
                        0.5 * (last.x + first.x),
                        0.5 * (last.y + first.y),
                        PointType::QCurve,
                        false,
                        None,
                        None,
                        None,
                    );
                    points.push(&implied_oncurve);
                }
            }
        }
    }

    let start = start.unwrap();
    segments.push(PathEl::MoveTo(Point::new(start.x as f64, start.y as f64)));

    // 1. Single-point contour: convert to moveto, done.
    // 2. Open contour: starts with move, use first point as starting moveto
    // 3. Closed contour: does not start with move, ...
    //   a. ...at least one on-curve: use last point (after rotation) as starting moveto
    //   b. ...all off-curves: quad blob contour; use last implied point as starting point (append it) but do not emit moveto.
    //
    // segments handling:
    //  move: must be 1 point
    //  line: must be 1 point
    //  offcurve:
    //      1 => unreachable if we return early above?
    //      n followed by oncurve => collect for oncurve
    //      else => unreachable after we expand implied qcurve above and for closed, but open contour could have illegal trailing offcurves
    //  qcurve:
    //      1 => convert to lineto
    //      2 => standard curve
    //      else => decomposeQuadraticSegment
    //  curve:
    //      1 => convert to lineto
    //      2 => convert to qcurve
    //      3 => standard curve
    //      else => decomposeSuperBezierSegment

    let mut controls: Vec<Point> = Vec::new();
    for point in points {
        let p = Point::new(point.x as f64, point.y as f64);
        match point.typ {
            PointType::OffCurve => controls.push(p),
            PointType::Move => return Err(ContourDrawingError::IllegalMove),
            PointType::Line => {
                if !controls.is_empty() {
                    return Err(ContourDrawingError::IllegalPointCount(
                        PointType::Line,
                        controls.len(),
                    ));
                }
                segments.push(PathEl::LineTo(p))
            }
            PointType::QCurve => match controls.len() {
                0 => segments.push(PathEl::LineTo(p)),
                1 => {
                    segments.push(PathEl::QuadTo(controls[0], p));
                    controls.clear()
                }
                _ => {
                    // TODO: make iterator?
                    for i in 0..=controls.len() - 2 {
                        let c = controls[i];
                        let cn = controls[i + 1];
                        let pi = Point::new(0.5 * (c.x + cn.x), 0.5 * (c.y + cn.y));
                        segments.push(PathEl::QuadTo(c, pi));
                    }
                    segments.push(PathEl::QuadTo(controls[controls.len() - 1], p));
                    controls.clear()
                }
            },
            PointType::Curve => match controls.len() {
                0 => segments.push(PathEl::LineTo(p)),
                1 => {
                    segments.push(PathEl::QuadTo(controls[0], p));
                    controls.clear()
                }
                2 => {
                    segments.push(PathEl::CurveTo(controls[0], controls[1], p));
                    controls.clear()
                }
                _ => todo!(),
            },
        }
    }
    if !controls.is_empty() {
        return Err(ContourDrawingError::TrailingOffCurves);
    }
    if closed {
        segments.push(PathEl::ClosePath);
    }

    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_mutatorsans() {
        let ufo = norad::Ufo::load("testdata/mutatorSans/MutatorSansLightWide.ufo").unwrap();
        let default_layer = ufo.get_default_layer().unwrap();

        let units_per_em: f64 =
            ufo.font_info.as_ref().map_or(1000.0, |info| match info.units_per_em {
                Some(a) => a.get(),
                None => 1000.0,
            });
        let angle: f64 = ufo.font_info.as_ref().map_or(0.0, |info| match info.italic_angle {
            Some(a) => -a.get(),
            None => 0.0,
        });
        let xheight: f64 = ufo.font_info.as_ref().map_or(0.0, |info| match info.x_height {
            Some(a) => a.get(),
            None => 0.0,
        });
        let param_area: f64 = 400.0;
        let param_depth: f64 = 15.0;
        let param_overshoot: f64 = 0.0;
        let overshoot = xheight * param_overshoot / 100.0;
        let param_sample_frequency: usize = 5;

        let mut background_glyphs = Vec::new();

        for (name, left, right) in &[
            ("A", Some(31.0), Some(31.0)),
            ("acute", Some(79.0), Some(79.0)),
            ("B", Some(100.0), Some(51.0)),
            ("C", Some(57.0), Some(51.0)),
            ("D", Some(100.0), Some(57.0)),
            ("E", Some(100.0), Some(41.0)),
            ("F", Some(100.0), Some(40.0)),
            ("G", Some(57.0), Some(74.0)),
            ("H", Some(100.0), Some(100.0)),
            ("I", Some(41.0), Some(41.0)),
            ("I.narrow", Some(100.0), Some(100.0)),
            ("IJ", Some(79.0), Some(80.0)),
            ("J", Some(49.0), Some(83.0)),
            ("J.narrow", Some(32.0), Some(80.0)),
            ("K", Some(100.0), Some(33.0)),
            ("L", Some(100.0), Some(33.0)),
            ("M", Some(100.0), Some(100.0)),
            ("N", Some(100.0), Some(100.0)),
            ("O", Some(57.0), Some(57.0)),
            ("P", Some(100.0), Some(54.0)),
            ("R", Some(100.0), Some(57.0)),
            ("S", Some(46.0), Some(52.0)),
            ("S.closed", Some(51.0), Some(50.0)),
            ("T", Some(33.0), Some(33.0)),
            ("U", Some(80.0), Some(80.0)),
            ("V", Some(31.0), Some(31.0)),
            ("W", Some(34.0), Some(34.0)),
            ("X", Some(27.0), Some(33.0)),
            ("Y", Some(30.0), Some(30.0)),
            ("Z", Some(41.0), Some(41.0)),
            ("arrowdown", Some(89.0), Some(91.0)),
            ("arrowleft", Some(95.0), Some(111.0)),
            ("arrowright", Some(110.0), Some(96.0)),
            ("arrowup", Some(91.0), Some(89.0)),
            ("period", Some(112.0), Some(112.0)),
            ("comma", Some(110.0), Some(107.0)),
            ("dot", Some(80.0), Some(80.0)),
            ("Aacute", Some(31.0), Some(31.0)),
            ("Q", Some(57.0), Some(57.0)),
            ("colon", Some(104.0), Some(104.0)),
            ("quotedblbase", Some(94.0), Some(91.0)),
            ("quotedblleft", Some(91.0), Some(94.0)),
            ("quotedblright", Some(94.0), Some(91.0)),
            ("quotesinglbase", Some(94.0), Some(91.0)),
            ("semicolon", Some(104.0), Some(102.0)),
            ("dieresis", Some(80.0), Some(80.0)),
            ("Adieresis", Some(31.0), Some(31.0)),
            ("space", None, None),
        ] {
            let glyph = ufo.get_glyph(*name).unwrap();

            let (mut factor, glyph_ref) = config_for_glyph(&glyph, &default_layer);
            if &*glyph.name == "dot" {
                factor = 1.0;
            }

            let paths = path_for_glyph(&glyph, &default_layer).unwrap().unwrap();
            let bounds = paths.bounding_box();
            let paths_reference = path_for_glyph(&glyph_ref, &default_layer).unwrap().unwrap();
            let bounds_reference = paths_reference.bounding_box();
            let bounds_reference_lower = (bounds_reference.min_y() - overshoot).round();
            let bounds_reference_upper = (bounds_reference.max_y() + overshoot).round();

            let (new_left, new_right) = calculate_spacing(
                paths,
                bounds,
                (bounds_reference_lower, bounds_reference_upper),
                angle,
                xheight,
                param_sample_frequency,
                param_depth,
                name.clone(),
                &mut background_glyphs,
                factor,
                param_area,
                units_per_em,
            );

            match (left, new_left) {
                (Some(v), Some(new_v)) => assert!(
                    (*v - new_v).abs() <= 1.0,
                    "Glyph {}: expected left {} but got {} (factor {})",
                    *name,
                    v,
                    new_v,
                    factor
                ),
                (None, None) => (),
                _ => assert!(
                    false,
                    "Glyph {}, left side: expected {:?}, got {:?} (factor {})",
                    *name, left, new_left, factor
                ),
            }
            match (right, new_right) {
                (Some(v), Some(new_v)) => assert!(
                    (*v - new_v).abs() <= 1.0,
                    "Glyph {}: expected right {} but got {} (factor {})",
                    *name,
                    v,
                    new_v,
                    factor
                ),
                (None, None) => (),
                _ => assert!(
                    false,
                    "Glyph {}, right side: expected {:?}, got {:?} (factor {})",
                    *name, right, new_right, factor
                ),
            }
        }
    }
}
