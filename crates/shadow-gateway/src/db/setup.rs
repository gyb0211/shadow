//! 首次初始化逻辑（JSON 文件存储）
//!
//! 用户数据存储在 ~/.shadow/users.json 文件中

use std::path::PathBuf;
use anyhow::Result;
use bcrypt::{hash, DEFAULT_COST};

use super::entities::{User, UserRole};
use super::UserStore as Store;

/// 用户存储文件路径
pub fn get_store_path(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("users.json")
}

/// 加载用户存储
fn load_store(path: &PathBuf) -> Result<Store> {
    if path.exists() {
        let content = std::fs::read_to_string(path)?;
        let store: Store = serde_json::from_str(&content)?;
        Ok(store)
    } else {
        Ok(Store::default())
    }
}

/// 保存用户存储
fn save_store(path: &PathBuf, store: &Store) -> Result<()> {
    // 确保目录存在
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(store)?;
    std::fs::write(path, content)?;
    Ok(())
}

/// 检查是否已初始化（有管理员账号）
pub fn is_initialized(data_dir: &PathBuf) -> Result<bool> {
    let path = get_store_path(data_dir);
    let store = load_store(&path)?;
    Ok(store.users.iter().any(|u| u.role == UserRole::Admin))
}

/// 创建管理员账号
pub fn create_admin(
    data_dir: &PathBuf,
    username: &str,
    password: &str,
) -> Result<User> {
    let path = get_store_path(data_dir);
    let mut store = load_store(&path)?;

    // 检查用户名是否已存在
    if store.users.iter().any(|u| u.username == username) {
        anyhow::bail!("用户名已存在");
    }

    let password_hash = hash(password, DEFAULT_COST)?;
    let now = chrono::Utc::now();
    let id = store.users.len() as i32 + 1;

    let user = User {
        id,
        username: username.to_string(),
        password_hash,
        role: UserRole::Admin,
        created_at: now,
        updated_at: now,
    };

    store.users.push(user.clone());
    save_store(&path, &store)?;

    Ok(user)
}

/// 根据用户名查找用户
pub fn find_user_by_username(data_dir: &PathBuf, username: &str) -> Result<Option<User>> {
    let path = get_store_path(data_dir);
    let store = load_store(&path)?;
    Ok(store.users.into_iter().find(|u| u.username == username))
}

/// 验证密码
pub fn verify_password(data_dir: &PathBuf, username: &str, password: &str) -> Result<Option<User>> {
    let user = find_user_by_username(data_dir, username)?;
    
    if let Some(user) = user {
        if bcrypt::verify(password, &user.password_hash).unwrap_or(false) {
            return Ok(Some(user));
        }
    }
    
    Ok(None)
}

/// 列出所有用户
pub fn list_users(data_dir: &PathBuf) -> Result<Vec<User>> {
    let path = get_store_path(data_dir);
    let store = load_store(&path)?;
    Ok(store.users)
}

/// 创建用户
pub fn create_user(
    data_dir: &PathBuf,
    username: &str,
    password: &str,
    role: UserRole,
) -> Result<User> {
    let path = get_store_path(data_dir);
    let mut store = load_store(&path)?;

    // 检查用户名是否已存在
    if store.users.iter().any(|u| u.username == username) {
        anyhow::bail!("用户名已存在");
    }

    let password_hash = hash(password, DEFAULT_COST)?;
    let now = chrono::Utc::now();
    let id = store.users.len() as i32 + 1;

    let user = User {
        id,
        username: username.to_string(),
        password_hash,
        role,
        created_at: now,
        updated_at: now,
    };

    store.users.push(user.clone());
    save_store(&path, &store)?;

    Ok(user)
}

/// 删除用户
pub fn delete_user(data_dir: &PathBuf, id: i32) -> Result<()> {
    let path = get_store_path(data_dir);
    let mut store = load_store(&path)?;
    store.users.retain(|u| u.id != id);
    save_store(&path, &store)?;
    Ok(())
}
