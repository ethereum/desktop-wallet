use std::fmt::Write;

pub fn table(headers: &[&str], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }

    let render = |cells: &mut dyn Iterator<Item = &str>| {
        let mut line = String::new();
        for (cell, &width) in cells.zip(&widths) {
            let _ = write!(line, "{cell:<width$}  ");
        }
        println!("{}", line.trim_end());
    };

    render(&mut headers.iter().copied());
    for row in rows {
        render(&mut row.iter().map(String::as_str));
    }
}
