use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillBundleConfig {
    ///从那里加载skills 相对于工作区的文件夹路径
    pub directory: Option<String>,
    /// 包含哪些skill(empty == all)
    pub include: Vec<String>,
    /// 排除哪些skill
    pub exclude: Vec<String>,
}

impl SkillBundleConfig {
    pub fn admits_skill(&self, name: &str) -> bool {
        if !self.include.is_empty() && !self.include.contains(&name.to_string()) {  return false;}
        !self.exclude.contains(&name.to_string())
    }
}
