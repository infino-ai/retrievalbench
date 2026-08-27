// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! VectorDBBench Cohere ground-truth reader.

use std::{fs::File, path::Path};

use arrow_array::{
    Array, Int32Array, Int64Array, LargeListArray, ListArray, UInt32Array, UInt64Array,
};
use infino_bench_utils::corpus::CorpusSource;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

const NEIGHBORS_COLUMN: &str = "neighbors_id";

fn primitive_ids(values: &dyn Array) -> Vec<u32> {
    if let Some(values) = values.as_any().downcast_ref::<Int64Array>() {
        return values
            .iter()
            .flatten()
            .map(|id| u32::try_from(id).expect("non-negative Cohere neighbor id"))
            .collect();
    }
    if let Some(values) = values.as_any().downcast_ref::<UInt64Array>() {
        return values
            .iter()
            .flatten()
            .map(|id| u32::try_from(id).expect("Cohere neighbor id fits u32"))
            .collect();
    }
    if let Some(values) = values.as_any().downcast_ref::<Int32Array>() {
        return values
            .iter()
            .flatten()
            .map(|id| u32::try_from(id).expect("non-negative Cohere neighbor id"))
            .collect();
    }
    if let Some(values) = values.as_any().downcast_ref::<UInt32Array>() {
        return values.iter().flatten().collect();
    }
    panic!(
        "unsupported Cohere neighbor value type: {:?}",
        values.data_type()
    );
}

pub fn ground_truth(
    source: &CorpusSource,
    n_docs: usize,
    n_queries: usize,
    k: usize,
) -> Option<Vec<Vec<u32>>> {
    let CorpusSource::LocalParquet { dir } = source else {
        return None;
    };
    let path = dir.join("ground-truth").join(format!("{n_docs}.parquet"));
    if !path.exists() {
        return None;
    }
    Some(read_ground_truth(&path, n_queries, k))
}

fn read_ground_truth(path: &Path, n_queries: usize, k: usize) -> Vec<Vec<u32>> {
    let file = File::open(path).expect("open Cohere ground truth");
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("read Cohere ground-truth metadata")
        .with_batch_size(n_queries)
        .build()
        .expect("build Cohere ground-truth reader");
    let mut rows = Vec::with_capacity(n_queries);
    for batch in reader {
        let batch = batch.expect("read Cohere ground-truth batch");
        let column = batch
            .column_by_name(NEIGHBORS_COLUMN)
            .expect("Cohere neighbors_id column");
        if let Some(lists) = column.as_any().downcast_ref::<ListArray>() {
            for row in 0..lists.len() {
                let ids = primitive_ids(lists.value(row).as_ref());
                rows.push(ids.into_iter().take(k).collect());
                if rows.len() == n_queries {
                    return rows;
                }
            }
        } else if let Some(lists) = column.as_any().downcast_ref::<LargeListArray>() {
            for row in 0..lists.len() {
                let ids = primitive_ids(lists.value(row).as_ref());
                rows.push(ids.into_iter().take(k).collect());
                if rows.len() == n_queries {
                    return rows;
                }
            }
        } else {
            panic!(
                "unsupported Cohere neighbors_id type: {:?}",
                column.data_type()
            );
        }
    }
    assert_eq!(
        rows.len(),
        n_queries,
        "Cohere ground truth has too few queries"
    );
    rows
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::types::Int64Type;
    use arrow_array::{ArrayRef, ListArray, RecordBatch};
    use arrow_schema::{Field, Schema};
    use parquet::arrow::ArrowWriter;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn reads_vectordbbench_neighbor_lists() {
        let dir = tempdir().expect("ground-truth tempdir");
        let gt_dir = dir.path().join("ground-truth");
        std::fs::create_dir(&gt_dir).expect("create ground-truth directory");
        let path = gt_dir.join("10.parquet");
        let lists = ListArray::from_iter_primitive::<Int64Type, _, _>(vec![
            Some(vec![Some(4), Some(2), Some(8)]),
            Some(vec![Some(1), Some(7), Some(3)]),
        ]);
        let schema = Arc::new(Schema::new(vec![Field::new(
            NEIGHBORS_COLUMN,
            lists.data_type().clone(),
            false,
        )]));
        let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(lists) as ArrayRef])
            .expect("ground-truth batch");
        let file = File::create(&path).expect("create ground-truth parquet");
        let mut writer = ArrowWriter::try_new(file, schema, None).expect("ground-truth writer");
        writer.write(&batch).expect("write ground truth");
        writer.close().expect("close ground truth");

        let source = CorpusSource::LocalParquet {
            dir: dir.path().to_path_buf(),
        };
        let rows = ground_truth(&source, 10, 2, 2).expect("read ground truth");
        assert_eq!(rows, vec![vec![4, 2], vec![1, 7]]);
    }

    #[test]
    #[ignore = "requires COHERE_GT_PATH pointing at a downloaded VectorDBBench file"]
    fn reads_public_vectordbbench_file() {
        let path = std::env::var("COHERE_GT_PATH").expect("COHERE_GT_PATH");
        let rows = read_ground_truth(Path::new(&path), 200, 100);
        assert_eq!(rows.len(), 200);
        assert!(rows.iter().all(|row| row.len() == 100));
    }
}
