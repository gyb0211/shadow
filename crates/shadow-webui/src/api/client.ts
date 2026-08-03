/**
 * API 客户端 - 与 shadow-gateway 通信
 */
import axios, { type AxiosError, type InternalAxiosRequestConfig } from 'axios';

const API_BASE = '/api';

export interface LoginRequest {
  username: string;
  password: string;
}

export interface LoginResponse {
  token: string;
  user: UserInfo;
}

export type DatabaseType = 'sqlite' | 'mysql';

export interface SqliteDatabaseConfig {
  type: 'sqlite';
  path?: string;
}

export interface MysqlDatabaseConfig {
  type: 'mysql';
  host: string;
  port: number;
  user: string;
  password: string;
  database: string;
}

export type DatabaseConfig = SqliteDatabaseConfig | MysqlDatabaseConfig;

export interface SetupRequest {
  database: DatabaseConfig;
  admin: {
    username: string;
    password: string;
  };
}

export interface SetupStatusResponse {
  initialized: boolean;
}

export interface UserInfo {
  id: number;
  username: string;
  role: 'admin' | 'viewer';
}

export interface StatusResponse {
  version: string;
  config_path: string;
  data_dir: string;
  daemon_running: boolean;
}

// 创建 axios 实例
const client = axios.create({
  baseURL: API_BASE,
  timeout: 30000,
});

// 请求拦截器：添加 JWT token
client.interceptors.request.use((config: InternalAxiosRequestConfig) => {
  const token = localStorage.getItem('shadow_token');
  if (token && config.headers) {
    config.headers.Authorization = `Bearer ${token}`;
  }
  return config;
});

// 响应拦截器：处理 401
client.interceptors.response.use(
  (response) => response,
  (error: AxiosError) => {
    if (error.response?.status === 401) {
      localStorage.removeItem('shadow_token');
      window.location.href = '/login';
    }
    return Promise.reject(error);
  }
);

// 认证相关 API
export const authApi = {
  // 获取初始化状态
  getSetupStatus: async (): Promise<SetupStatusResponse> => {
    const res = await client.get('/auth/setup/status');
    return res.data;
  },

  // 首次初始化
  setup: async (data: SetupRequest): Promise<LoginResponse> => {
    const res = await client.post('/auth/setup', data);
    return res.data;
  },

  // 登录
  login: async (data: LoginRequest): Promise<LoginResponse> => {
    const res = await client.post('/auth/login', data);
    return res.data;
  },

  // 获取当前用户
  me: async (): Promise<UserInfo> => {
    const res = await client.get('/auth/me');
    return res.data;
  },
};

// 系统状态 API
export const statusApi = {
  get: async (): Promise<StatusResponse> => {
    const res = await client.get('/status');
    return res.data;
  },
};

// 用户管理 API (admin only)
export const usersApi = {
  list: async (): Promise<UserInfo[]> => {
    const res = await client.get('/users');
    return res.data;
  },

  create: async (data: { username: string; password: string; role: 'admin' | 'viewer' }): Promise<UserInfo> => {
    const res = await client.post('/users', data);
    return res.data;
  },

  delete: async (id: number): Promise<void> => {
    await client.delete(`/users/${id}`);
  },
};

// Agent 配置 API
export interface AgentConfig {
  alias: string;
  model?: string;
  provider?: string;
  tools?: string[];
  channels?: string[];
  system_prompt?: string;
  max_tokens?: number;
  temperature?: number;
}

export const agentsApi = {
  list: async (): Promise<AgentConfig[]> => {
    const res = await client.get('/agents');
    return res.data;
  },

  get: async (alias: string): Promise<AgentConfig> => {
    const res = await client.get(`/agents/${alias}`);
    return res.data;
  },

  update: async (alias: string, data: Partial<AgentConfig>): Promise<AgentConfig> => {
    const res = await client.put(`/agents/${alias}`, data);
    return res.data;
  },
};

// Channel 配置 API
export interface ChannelConfig {
  type: string;
  alias: string;
  enabled: boolean;
  config: Record<string, unknown>;
}

export const channelsApi = {
  list: async (): Promise<ChannelConfig[]> => {
    const res = await client.get('/channels');
    return res.data;
  },

  update: async (type: string, alias: string, data: Partial<ChannelConfig>): Promise<ChannelConfig> => {
    const res = await client.put(`/channels/${type}/${alias}`, data);
    return res.data;
  },
};

// Provider 配置 API
export interface ProviderConfig {
  type: string;
  alias: string;
  enabled: boolean;
  config: Record<string, unknown>;
}

export const providersApi = {
  list: async (): Promise<ProviderConfig[]> => {
    const res = await client.get('/providers');
    return res.data;
  },

  update: async (type: string, alias: string, data: Partial<ProviderConfig>): Promise<ProviderConfig> => {
    const res = await client.put(`/providers/${type}/${alias}`, data);
    return res.data;
  },
};

// 工具列表 API
export interface ToolInfo {
  name: string;
  description: string;
  parameters?: Record<string, unknown>;
}

export const toolsApi = {
  list: async (): Promise<ToolInfo[]> => {
    const res = await client.get('/tools');
    return res.data;
  },
};

// 配置管理 API
export const configApi = {
  get: async (): Promise<Record<string, unknown>> => {
    const res = await client.get('/config');
    return res.data;
  },

  update: async (data: Record<string, unknown>): Promise<Record<string, unknown>> => {
    const res = await client.put('/config', data);
    return res.data;
  },
};

// 日志 API
export interface LogEntry {
  timestamp: string;
  level: string;
  message: string;
  target?: string;
}

export const logsApi = {
  list: async (params?: { level?: string; limit?: number }): Promise<LogEntry[]> => {
    const res = await client.get('/logs', { params });
    return res.data;
  },
};

export default client;
