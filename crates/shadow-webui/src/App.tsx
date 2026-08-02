/**
 * Shadow WebUI 主应用
 * React + TypeScript + Ant Design v5 + TanStack Query
 */
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { ConfigProvider } from 'antd';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

// 上下文
import { AuthProvider, useAuth } from './contexts/AuthContext';

// 页面组件
import LoginPage from './pages/LoginPage';
import SetupPage from './pages/SetupPage';
import DashboardPage from './pages/DashboardPage';
import AgentsPage from './pages/AgentsPage';
import ChannelsPage from './pages/ChannelsPage';
import ProvidersPage from './pages/ProvidersPage';
import ToolsPage from './pages/ToolsPage';
import ConfigPage from './pages/ConfigPage';
import LogsPage from './pages/LogsPage';
import UsersPage from './pages/UsersPage';

// 组件
import MainLayout from './components/MainLayout';
import ProtectedRoute from './components/ProtectedRoute';

// 创建 QueryClient
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30000,
      retry: 1,
    },
  },
});

// 内部路由组件（需要在 AuthProvider 内使用 useAuth）
function AppRoutes() {
  const { isAuthenticated } = useAuth();

  return (
    <Routes>
      {/* 公开路由 */}
      <Route path="/login" element={isAuthenticated ? <Navigate to="/" replace /> : <LoginPage />} />
      <Route path="/setup" element={<SetupPage />} />

      {/* 受保护的路由 */}
      <Route
        path="/"
        element={
          <ProtectedRoute>
            <MainLayout />
          </ProtectedRoute>
        }
      >
        <Route index element={<DashboardPage />} />
        <Route path="agents" element={<AgentsPage />} />
        <Route path="channels" element={<ChannelsPage />} />
        <Route path="providers" element={<ProvidersPage />} />
        <Route path="tools" element={<ToolsPage />} />
        <Route path="config" element={<ConfigPage />} />
        <Route path="logs" element={<LogsPage />} />
        <Route path="users" element={<UsersPage />} />
      </Route>

      {/* 未匹配的路由重定向到首页 */}
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}

// 根组件
function App() {
  return (
    <ConfigProvider
      theme={{
        token: {
          colorPrimary: '#722ed1',
        },
      }}
    >
      <QueryClientProvider client={queryClient}>
        <BrowserRouter>
          <AuthProvider>
            <AppRoutes />
          </AuthProvider>
        </BrowserRouter>
      </QueryClientProvider>
    </ConfigProvider>
  );
}

export default App;
