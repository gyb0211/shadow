/**
 * 主布局组件 -- 深色侧边栏(全高) + 玻璃拟态头部 + flex 内容区
 */
import { useState } from 'react';
import { Layout, Menu, Avatar, Dropdown, Typography, Space, Tooltip } from 'antd';
import {
  DashboardOutlined,
  RobotOutlined,
  ApiOutlined,
  CloudServerOutlined,
  ToolOutlined,
  SettingOutlined,
  FileTextOutlined,
  TeamOutlined,
  LogoutOutlined,
  UserOutlined,
  MenuFoldOutlined,
  MenuUnfoldOutlined,
} from '@ant-design/icons';
import { Outlet, useNavigate, useLocation } from 'react-router-dom';
import { useAuth } from '../contexts/AuthContext';

const { Header, Sider, Content } = Layout;
const { Text } = Typography;

const menuItems = [
  { key: '/', icon: <DashboardOutlined />, label: '概览' },
  { key: '/agents', icon: <RobotOutlined />, label: 'Agent' },
  { key: '/channels', icon: <ApiOutlined />, label: 'Channel' },
  { key: '/providers', icon: <CloudServerOutlined />, label: 'Provider' },
  { key: '/tools', icon: <ToolOutlined />, label: '工具' },
  { key: '/config', icon: <SettingOutlined />, label: '配置' },
  { key: '/logs', icon: <FileTextOutlined />, label: '日志' },
  { key: '/users', icon: <TeamOutlined />, label: '用户', adminOnly: true },
];

export default function MainLayout() {
  const { user, logout } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const [collapsed, setCollapsed] = useState(false);
  const isAdmin = user?.role === 'admin';

  const filteredItems = menuItems.filter((item) => !item.adminOnly || isAdmin);

  const userMenu = {
    items: [
      { key: 'profile', icon: <UserOutlined />, label: `${user?.username || ''}`, disabled: true },
      { type: 'divider' as const },
      { key: 'logout', icon: <LogoutOutlined />, label: '退出登录', danger: true },
    ],
    onClick: ({ key }: { key: string }) => {
      if (key === 'logout') {
        logout();
        navigate('/login');
      }
    },
  };

  return (
    <Layout style={{ minHeight: '100vh' }}>
      {/* 侧边栏 -- 全高 fixed */}
      <Sider
        theme="dark"
        width={220}
        collapsedWidth={64}
        collapsible
        collapsed={collapsed}
        onCollapse={setCollapsed}
        trigger={null}
        style={{
          position: 'fixed',
          left: 0,
          top: 0,
          bottom: 0,
          zIndex: 10,
          borderRight: '1px solid rgba(255,255,255,0.06)',
          display: 'flex',
          flexDirection: 'column',
        }}
      >
        {/* Logo 区 */}
        <div style={{
          height: 56,
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: collapsed ? '0 20px' : '0 20px',
          borderBottom: '1px solid rgba(255,255,255,0.06)',
          overflow: 'hidden',
          whiteSpace: 'nowrap',
          flexShrink: 0,
        }}>
          <div style={{
            width: 28,
            height: 28,
            minWidth: 28,
            borderRadius: 8,
            background: 'linear-gradient(135deg, #6c5ce7, #00cec9)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            fontSize: 14,
            fontWeight: 800,
            color: '#fff',
          }}>
            S
          </div>
          {!collapsed && (
            <span style={{ color: '#fff', fontSize: 16, fontWeight: 600, letterSpacing: 0.5 }}>
              Shadow
            </span>
          )}
        </div>

        {/* 菜单区 -- flex 撑满 */}
        <div style={{ flex: 1, overflow: 'auto', paddingTop: 8 }}>
          <Menu
            theme="dark"
            mode="inline"
            selectedKeys={[location.pathname]}
            items={filteredItems}
            onClick={({ key }) => navigate(key)}
            style={{ border: 'none' }}
          />
        </div>

        {/* 底部版本区 */}
        {!collapsed && (
          <div style={{
            padding: '12px 20px',
            borderTop: '1px solid rgba(255,255,255,0.06)',
            flexShrink: 0,
          }}>
            <Text style={{ color: 'rgba(255,255,255,0.2)', fontSize: 11 }}>
              Shadow v0.1.0
            </Text>
          </div>
        )}
      </Sider>

      {/* 右侧主区域 */}
      <Layout style={{ marginLeft: collapsed ? 64 : 220, transition: 'margin-left 0.2s' }}>
        {/* 头部 -- sticky 玻璃拟态 */}
        <Header style={{
          position: 'sticky',
          top: 0,
          zIndex: 9,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          backdropFilter: 'blur(12px)',
          borderBottom: '1px solid rgba(255,255,255,0.06)',
          flexShrink: 0,
        }}>
          <Tooltip title={collapsed ? '展开' : '折叠'}>
            <div
              onClick={() => setCollapsed(!collapsed)}
              style={{ cursor: 'pointer', fontSize: 16, color: 'rgba(255,255,255,0.45)', padding: '0 4px' }}
            >
              {collapsed ? <MenuUnfoldOutlined /> : <MenuFoldOutlined />}
            </div>
          </Tooltip>

          <Space size={12}>
            <Text style={{ color: 'rgba(255,255,255,0.45)', fontSize: 13 }}>
              {user?.username}
            </Text>
            <span style={{
              padding: '2px 8px',
              borderRadius: 4,
              fontSize: 11,
              fontWeight: 500,
              background: isAdmin ? 'rgba(255,118,117,0.15)' : 'rgba(0,206,201,0.15)',
              color: isAdmin ? '#ff7675' : '#00cec9',
            }}>
              {isAdmin ? '管理员' : '查看者'}
            </span>
            <Dropdown menu={userMenu} placement="bottomRight">
              <Avatar
                size={32}
                style={{
                  cursor: 'pointer',
                  background: 'linear-gradient(135deg, #6c5ce7, #a29bfe)',
                }}
                icon={<UserOutlined />}
              />
            </Dropdown>
          </Space>
        </Header>

        {/* 内容区 -- flex 撑满剩余高度 */}
        <Content style={{
          flex: 1,
          padding: 24,
          overflow: 'auto',
          display: 'flex',
          flexDirection: 'column',
        }}>
          <div className="page-enter" style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <Outlet />
          </div>
        </Content>
      </Layout>
    </Layout>
  );
}
