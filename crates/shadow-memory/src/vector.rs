//! 向量操作 -- 余弦相似度, 序列化, 混合检索融合
//!
//! 提供:
//! - [`cosine_similarity`] — 计算两个向量的余弦相似度 (0.0–1.0)
//! - [`vec_to_bytes`] / [`bytes_to_vec`] — f32 向量与字节序列互转 (用于 SQLite BLOB 存储)
//! - [`hybrid_merge`] — 加权融合向量检索与关键词检索结果

/// 计算两个向量的余弦相似度, 返回 0.0–1.0
///
/// 使用 f64 中间累加以减少精度损失, 最终 clamp 到 [0, 1]。
/// 长度不匹配、空向量、零向量或非有限值均返回 0.0。
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;

    for (x, y) in a.iter().zip(b.iter()) {
        let x = f64::from(*x);
        let y = f64::from(*y);
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if !denom.is_finite() || denom < f64::EPSILON {
        return 0.0;
    }

    let raw = dot / denom;
    if !raw.is_finite() {
        return 0.0;
    }

    // 截断到 [0, 1] -- embedding 向量通常为正值
    #[allow(clippy::cast_possible_truncation)]
    let sim = raw.clamp(0.0, 1.0) as f32;
    sim
}

/// 将 f32 向量序列化为字节数组 (小端序)
pub fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for &f in v {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

/// 将字节数组反序列化为 f32 向量 (小端序)
pub fn bytes_to_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
            f32::from_le_bytes(arr)
        })
        .collect()
}

/// 混合检索结果 -- 包含向量分数、关键词分数和最终融合分数
#[derive(Debug, Clone)]
pub struct ScoredResult {
    /// 记忆条目 ID
    pub id: String,
    /// 向量检索分数 (余弦相似度, 0.0–1.0)
    pub vector_score: Option<f32>,
    /// 关键词检索分数 (归一化到 0.0–1.0)
    pub keyword_score: Option<f32>,
    /// 融合后的最终分数
    pub final_score: f32,
}

/// 混合融合: 将向量检索与关键词检索结果加权合并
///
/// 归一化每组分数到 [0, 1], 然后计算:
///   `final_score` = `vector_weight` * `vector_score` + `keyword_weight` * `keyword_score`
///
/// 按 ID 去重, 保留每路的最佳分数, 最终按 final_score 降序排列并截断到 limit 条。
pub fn hybrid_merge(
    vector_results: &[(String, f32)],  // (id, 余弦相似度)
    keyword_results: &[(String, f32)], // (id, BM25 分数)
    vector_weight: f32,
    keyword_weight: f32,
    limit: usize,
) -> Vec<ScoredResult> {
    use std::collections::HashMap;

    let mut map: HashMap<String, ScoredResult> = HashMap::new();

    // 向量分数已经是 0–1 范围, 直接使用
    for (id, score) in vector_results {
        map.entry(id.clone())
            .and_modify(|r| r.vector_score = Some(*score))
            .or_insert_with(|| ScoredResult {
                id: id.clone(),
                vector_score: Some(*score),
                keyword_score: None,
                final_score: 0.0,
            });
    }

    // 关键词分数 (BM25 可能为任意正数), 归一化到 0–1
    let max_kw = keyword_results
        .iter()
        .map(|(_, s)| *s)
        .fold(0.0_f32, f32::max);
    let max_kw = if max_kw < f32::EPSILON { 1.0 } else { max_kw };

    for (id, score) in keyword_results {
        let normalized = score / max_kw;
        map.entry(id.clone())
            .and_modify(|r| r.keyword_score = Some(normalized))
            .or_insert_with(|| ScoredResult {
                id: id.clone(),
                vector_score: None,
                keyword_score: Some(normalized),
                final_score: 0.0,
            });
    }

    // 计算最终融合分数
    let mut results: Vec<ScoredResult> = map
        .into_values()
        .map(|mut r| {
            let vs = r.vector_score.unwrap_or(0.0);
            let ks = r.keyword_score.unwrap_or(0.0);
            r.final_score = vector_weight * vs + keyword_weight * ks;
            r
        })
        .collect();

    // 按最终分数降序排列, 同分按 ID 排序保持稳定
    results.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    results.truncate(limit);
    results
}

