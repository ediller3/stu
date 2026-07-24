use bytes::Bytes;
use parquet::{
    basic::Type as PhysicalType,
    file::reader::{FileReader, SerializedFileReader},
    record::Field,
    schema::types::Type as SchemaType,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Stylize,
    text::Line,
    widgets::{Block, StatefulWidget},
};

use crate::{
    color::Theme,
    environment::Environment,
    format::format_version,
    widget::{ScrollLines, ScrollLinesOptions, ScrollLinesState},
};

// cells wider than this are truncated with an ellipsis
const MAX_CELL_WIDTH: usize = 50;

#[derive(Debug)]
pub struct ParquetPreviewState {
    pub scroll_lines_state: ScrollLinesState,
    content: String,
}

impl ParquetPreviewState {
    pub fn new(bytes: &[u8], max_rows: usize) -> Result<Self, String> {
        let table = ParquetTable::read(bytes, max_rows)?;
        let table_lines = table.build_lines();
        let content = table_lines.join("\n");

        let mut lines: Vec<Line<'static>> = table_lines.into_iter().map(Line::raw).collect();
        if let Some(header_line) = lines.first_mut() {
            *header_line = std::mem::take(header_line).bold();
        }

        // wrapping and line numbers break the table layout, so disable them by default
        let options = ScrollLinesOptions::new(false, false);
        let state = Self {
            scroll_lines_state: ScrollLinesState::new(lines, options),
            content,
        };
        Ok(state)
    }

    pub fn content(&self) -> &str {
        &self.content
    }
}

struct ParquetColumn {
    name: String,
    numeric: bool,
}

struct ParquetTable {
    columns: Vec<ParquetColumn>,
    rows: Vec<Vec<String>>,
    not_shown_rows: usize,
}

impl ParquetTable {
    fn read(bytes: &[u8], max_rows: usize) -> Result<Self, String> {
        let bytes = Bytes::copy_from_slice(bytes);
        let reader = SerializedFileReader::new(bytes)
            .map_err(|e| format!("Failed to read parquet file: {e}"))?;

        let file_metadata = reader.metadata().file_metadata();
        let columns: Vec<ParquetColumn> = file_metadata
            .schema()
            .get_fields()
            .iter()
            .map(|f| ParquetColumn {
                name: f.name().to_string(),
                numeric: is_numeric_column(f),
            })
            .collect();
        if columns.is_empty() {
            return Err("Parquet file has no columns".to_string());
        }
        let total_rows = usize::try_from(file_metadata.num_rows()).unwrap_or_default();

        let max_rows = if max_rows == 0 { usize::MAX } else { max_rows };
        let row_iter = reader
            .get_row_iter(None)
            .map_err(|e| format!("Failed to read parquet rows: {e}"))?;
        let mut rows: Vec<Vec<String>> = Vec::new();
        for row in row_iter.take(max_rows) {
            let row = row.map_err(|e| format!("Failed to read parquet rows: {e}"))?;
            let cells = row
                .get_column_iter()
                .map(|(_, field)| format_field(field))
                .collect();
            rows.push(cells);
        }
        let not_shown_rows = total_rows.saturating_sub(rows.len());

        Ok(Self {
            columns,
            rows,
            not_shown_rows,
        })
    }

    fn build_lines(&self) -> Vec<String> {
        let widths: Vec<usize> = self
            .columns
            .iter()
            .enumerate()
            .map(|(i, col)| {
                self.rows
                    .iter()
                    .map(|row| cell_width(row, i))
                    .max()
                    .unwrap_or_default()
                    .max(console::measure_text_width(&col.name))
            })
            .collect();

        let mut lines: Vec<String> = Vec::with_capacity(self.rows.len() + 3);

        let header = self
            .columns
            .iter()
            .zip(&widths)
            .map(|(col, w)| pad_cell(&col.name, *w, false))
            .collect::<Vec<String>>()
            .join(" │ ");
        lines.push(header);

        let divider = widths
            .iter()
            .map(|w| "─".repeat(*w))
            .collect::<Vec<String>>()
            .join("─┼─");
        lines.push(divider);

        for row in &self.rows {
            let line = self
                .columns
                .iter()
                .zip(&widths)
                .enumerate()
                .map(|(i, (col, w))| {
                    let cell = row.get(i).map(String::as_str).unwrap_or_default();
                    pad_cell(cell, *w, col.numeric)
                })
                .collect::<Vec<String>>()
                .join(" │ ");
            lines.push(line);
        }

        if self.not_shown_rows > 0 {
            let unit = if self.not_shown_rows == 1 {
                "row"
            } else {
                "rows"
            };
            lines.push(format!("… ({} more {})", self.not_shown_rows, unit));
        }

        lines
    }
}

fn is_numeric_column(t: &SchemaType) -> bool {
    t.is_primitive()
        && matches!(
            t.get_physical_type(),
            PhysicalType::INT32
                | PhysicalType::INT64
                | PhysicalType::INT96
                | PhysicalType::FLOAT
                | PhysicalType::DOUBLE
        )
}

fn format_field(field: &Field) -> String {
    let s = match field {
        Field::Null => String::new(),
        Field::Str(s) => s.clone(),
        _ => field.to_string(),
    };
    let s: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    console::truncate_str(&s, MAX_CELL_WIDTH, "…").into_owned()
}

fn cell_width(row: &[String], i: usize) -> usize {
    row.get(i)
        .map(|cell| console::measure_text_width(cell))
        .unwrap_or_default()
}

fn pad_cell(s: &str, width: usize, right_align: bool) -> String {
    let pad = " ".repeat(width.saturating_sub(console::measure_text_width(s)));
    if right_align {
        format!("{pad}{s}")
    } else {
        format!("{s}{pad}")
    }
}

#[derive(Debug)]
pub struct ParquetPreview<'a> {
    file_name: &'a str,
    file_version_id: Option<&'a str>,

    env: &'a Environment,
    theme: &'a Theme,
}

impl<'a> ParquetPreview<'a> {
    pub fn new(
        file_name: &'a str,
        file_version_id: Option<&'a str>,
        env: &'a Environment,
        theme: &'a Theme,
    ) -> Self {
        Self {
            file_name,
            file_version_id,
            env,
            theme,
        }
    }
}

impl StatefulWidget for ParquetPreview<'_> {
    type State = ParquetPreviewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let title = if let Some(version_id) = self.file_version_id {
            format!(
                "Preview [{} (Version ID: {})]",
                self.file_name,
                format_version(Some(version_id), self.env.fix_dynamic_values)
            )
        } else {
            format!("Preview [{}]", self.file_name)
        };
        ScrollLines::default()
            .block(Block::bordered().title(title))
            .theme(self.theme)
            .render(area, buf, &mut state.scroll_lines_state);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parquet::{
        data_type::{ByteArray, ByteArrayType, Int64Type},
        file::{properties::WriterProperties, writer::SerializedFileWriter},
        schema::parser::parse_message_type,
    };

    use super::*;

    fn parquet_bytes(n: i64) -> Vec<u8> {
        let schema = Arc::new(
            parse_message_type(
                "message schema {
                    REQUIRED INT64 id;
                    REQUIRED BYTE_ARRAY name (UTF8);
                }",
            )
            .unwrap(),
        );
        let props = Arc::new(WriterProperties::builder().build());
        let mut bytes = Vec::new();
        let mut writer = SerializedFileWriter::new(&mut bytes, schema, props).unwrap();
        let mut row_group_writer = writer.next_row_group().unwrap();

        let mut col_writer = row_group_writer.next_column().unwrap().unwrap();
        let ids: Vec<i64> = (1..=n).collect();
        col_writer
            .typed::<Int64Type>()
            .write_batch(&ids, None, None)
            .unwrap();
        col_writer.close().unwrap();

        let mut col_writer = row_group_writer.next_column().unwrap().unwrap();
        let names: Vec<ByteArray> = (1..=n)
            .map(|i| format!("name-{i}").as_str().into())
            .collect();
        col_writer
            .typed::<ByteArrayType>()
            .write_batch(&names, None, None)
            .unwrap();
        col_writer.close().unwrap();

        row_group_writer.close().unwrap();
        writer.close().unwrap();

        bytes
    }

    #[test]
    fn test_parquet_preview_state_content() {
        let bytes = parquet_bytes(3);
        let state = ParquetPreviewState::new(&bytes, 10000).unwrap();

        let expected = [
            "id │ name  ",
            "───┼───────",
            " 1 │ name-1",
            " 2 │ name-2",
            " 3 │ name-3",
        ]
        .join("\n");
        assert_eq!(state.content(), expected);
    }

    #[test]
    fn test_parquet_preview_state_max_rows() {
        let bytes = parquet_bytes(10);

        let state = ParquetPreviewState::new(&bytes, 2).unwrap();
        let last_line = state.content().lines().last().unwrap().to_string();
        assert_eq!(state.content().lines().count(), 5); // header + divider + 2 rows + note
        assert_eq!(last_line, "… (8 more rows)");

        // 0 means unlimited
        let state = ParquetPreviewState::new(&bytes, 0).unwrap();
        assert_eq!(state.content().lines().count(), 12); // header + divider + 10 rows
    }

    #[test]
    fn test_parquet_preview_state_invalid_bytes() {
        let bytes = b"PAR1 this is not a valid parquet file PAR1";
        assert!(ParquetPreviewState::new(bytes, 10000).is_err());
    }
}
