/**
 * 首次初始化引导页面 -- 全屏渐变背景 + 玻璃拟态卡片
 */
import { useState, useEffect } from 'react';
import { Form, Input, Button, Steps, message, Typography } from 'antd';
import {
  DatabaseOutlined,
  UserOutlined,
  LockOutlined,
  CheckCircleOutlined,
  ArrowLeftOutlined,
  ArrowRightOutlined,
} from '@ant-design/icons';
import { useNavigate } from 'react-router-dom';
import { authApi, type DatabaseType } from '../api/client';

const { Title, Text, Paragraph } = Typography;

interface SetupFormData {
  dbType: DatabaseType;
  dbPath?: string;
  dbHost?: string;
  dbPort?: number;
  dbUser?: string;
  dbPassword?: string;
  dbDatabase?: string;
  adminUsername: string;
  adminPassword: string;
  adminConfirmPassword: string;
}

export default function SetupPage() {
  const [current, setCurrent] = useState(0);
  const [loading, setLoading] = useState(false);
  const [initialized, setInitialized] = useState(false);
  const [checking, setChecking] = useState(true);
  const [dbType, setDbType] = useState<DatabaseType>('sqlite');
  const [form] = Form.useForm<SetupFormData>();
  const navigate = useNavigate();

  useEffect(() => {
    const checkSetup = async () => {
      try {
        const status = await authApi.getSetupStatus();
        if (status.initialized) {
          setInitialized(true);
          navigate('/login');
        }
      } catch {
        // 未初始化
      } finally {
        setChecking(false);
      }
    };
    checkSetup();
  }, [navigate]);

  const onFinish = async () => {
    const values = form.getFieldsValue();

    if (values.adminPassword !== values.adminConfirmPassword) {
      message.error('两次密码输入不一致');
      return;
    }

    if (values.adminPassword.length < 6) {
      message.error('密码长度至少为 6 位');
      return;
    }

    setLoading(true);
    try {
      if (dbType === 'sqlite') {
        await authApi.setup({
          database: { type: 'sqlite', path: values.dbPath || 'gateway.db' },
          admin: { username: values.adminUsername, password: values.adminPassword },
        });
      } else {
        await authApi.setup({
          database: {
            type: 'mysql',
            host: values.dbHost || 'localhost',
            port: values.dbPort || 3306,
            user: values.dbUser || 'root',
            password: values.dbPassword || '',
            database: values.dbDatabase || 'shadow',
          },
          admin: { username: values.adminUsername, password: values.adminPassword },
        });
      }
      setCurrent(2);
    } catch (error: unknown) {
      const err = error as { response?: { data?: { message?: string } } };
      message.error(err.response?.data?.message || '初始化失败');
    } finally {
      setLoading(false);
    }
  };

  if (checking) {
    return (
      <div className="gradient-bg" style={{
        minHeight: '100vh',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}>
        <Text style={{ color: 'rgba(255,255,255,0.5)' }}>加载中...</Text>
      </div>
    );
  }

  if (initialized) return null;

  const steps = [
    { title: '数据库', icon: <DatabaseOutlined /> },
    { title: '管理员', icon: <UserOutlined /> },
    { title: '完成', icon: <CheckCircleOutlined /> },
  ];

  return (
    <div className="gradient-bg" style={{
      minHeight: '100vh',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      padding: 24,
    }}>
      {/* 背景装饰光晕 */}
      <div style={{
        position: 'fixed',
        top: '10%',
        left: '15%',
        width: 400,
        height: 400,
        borderRadius: '50%',
        background: 'radial-gradient(circle, rgba(108,92,231,0.15), transparent 70%)',
        pointerEvents: 'none',
      }} />
      <div style={{
        position: 'fixed',
        bottom: '10%',
        right: '15%',
        width: 350,
        height: 350,
        borderRadius: '50%',
        background: 'radial-gradient(circle, rgba(0,206,201,0.12), transparent 70%)',
        pointerEvents: 'none',
      }} />

      <div className="glass-card" style={{
        width: 520,
        maxWidth: '100%',
        padding: '40px 36px',
        position: 'relative',
        zIndex: 1,
      }}>
        {/* Logo */}
        <div style={{ textAlign: 'center', marginBottom: 28 }}>
          <div style={{
            width: 56,
            height: 56,
            borderRadius: 16,
            background: 'linear-gradient(135deg, #6c5ce7, #00cec9)',
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'center',
            fontSize: 28,
            fontWeight: 800,
            color: '#fff',
            marginBottom: 16,
            boxShadow: '0 8px 24px rgba(108,92,231,0.4)',
          }}>
            S
          </div>
          <Title level={3} style={{ color: '#fff', margin: 0 }}>Shadow 初始化</Title>
          <Paragraph style={{ color: 'rgba(255,255,255,0.45)', marginTop: 8 }}>
            首次使用需要配置数据库和管理员账号
          </Paragraph>
        </div>

        <Steps
          current={current}
          items={steps}
          size="small"
          style={{ marginBottom: 32 }}
        />

        {/* Step 0: 数据库配置 */}
        {current === 0 && (
          <Form form={form} layout="vertical" initialValues={{ dbType: 'sqlite', dbPath: 'gateway.db', dbPort: 3306 }}>
            <Text style={{ color: 'rgba(255,255,255,0.45)', fontSize: 13, display: 'block', marginBottom: 12 }}>
              选择数据库类型
            </Text>

            {/* 数据库类型卡片选择器 */}
            <div style={{ display: 'flex', gap: 12, marginBottom: 24 }}>
              <div
                onClick={() => setDbType('sqlite')}
                style={{
                  flex: 1,
                  padding: '16px 14px',
                  borderRadius: 12,
                  cursor: 'pointer',
                  border: dbType === 'sqlite'
                    ? '2px solid #6c5ce7'
                    : '1px solid rgba(255,255,255,0.08)',
                  background: dbType === 'sqlite'
                    ? 'rgba(108,92,231,0.08)'
                    : 'rgba(255,255,255,0.02)',
                  transition: 'all 0.2s',
                }}
              >
                <DatabaseOutlined style={{ fontSize: 20, color: dbType === 'sqlite' ? '#a29bfe' : 'rgba(255,255,255,0.3)' }} />
                <div style={{ color: '#fff', fontWeight: 600, marginTop: 8, fontSize: 14 }}>SQLite</div>
                <div style={{ color: 'rgba(255,255,255,0.35)', fontSize: 12, marginTop: 2 }}>轻量，无需额外服务</div>
              </div>

              <div
                onClick={() => setDbType('mysql')}
                style={{
                  flex: 1,
                  padding: '16px 14px',
                  borderRadius: 12,
                  cursor: 'pointer',
                  border: dbType === 'mysql'
                    ? '2px solid #6c5ce7'
                    : '1px solid rgba(255,255,255,0.08)',
                  background: dbType === 'mysql'
                    ? 'rgba(108,92,231,0.08)'
                    : 'rgba(255,255,255,0.02)',
                  transition: 'all 0.2s',
                }}
              >
                <DatabaseOutlined style={{ fontSize: 20, color: dbType === 'mysql' ? '#a29bfe' : 'rgba(255,255,255,0.3)' }} />
                <div style={{ color: '#fff', fontWeight: 600, marginTop: 8, fontSize: 14 }}>MySQL</div>
                <div style={{ color: 'rgba(255,255,255,0.35)', fontSize: 12, marginTop: 2 }}>生产环境推荐</div>
              </div>
            </div>

            {dbType === 'sqlite' && (
              <Form.Item label="数据库文件路径" name="dbPath" extra="相对于数据目录，默认 gateway.db">
                <Input placeholder="gateway.db" />
              </Form.Item>
            )}

            {dbType === 'mysql' && (
              <>
                <Form.Item label="主机地址" name="dbHost" rules={[{ required: true, message: '请输入主机地址' }]}>
                  <Input placeholder="localhost" />
                </Form.Item>
                <div style={{ display: 'flex', gap: 12 }}>
                  <Form.Item label="端口" name="dbPort" rules={[{ required: true, message: '请输入端口' }]} style={{ width: 120 }}>
                    <Input type="number" placeholder="3306" />
                  </Form.Item>
                  <Form.Item label="数据库名" name="dbDatabase" rules={[{ required: true, message: '请输入数据库名' }]} style={{ flex: 1 }}>
                    <Input placeholder="shadow" />
                  </Form.Item>
                </div>
                <Form.Item label="用户名" name="dbUser" rules={[{ required: true, message: '请输入用户名' }]}>
                  <Input placeholder="root" />
                </Form.Item>
                <Form.Item label="密码" name="dbPassword">
                  <Input.Password placeholder="留空则无密码" />
                </Form.Item>
              </>
            )}

            <Button
              type="primary"
              size="large"
              block
              onClick={() => setCurrent(1)}
              style={{ marginTop: 8 }}
            >
              下一步 <ArrowRightOutlined />
            </Button>
          </Form>
        )}

        {/* Step 1: 管理员账号 */}
        {current === 1 && (
          <Form form={form} layout="vertical">
            <Form.Item
              label="管理员用户名"
              name="adminUsername"
              rules={[{ required: true, message: '请输入管理员用户名' }]}
            >
              <Input prefix={<UserOutlined style={{ color: 'rgba(255,255,255,0.3)' }} />} placeholder="admin" size="large" />
            </Form.Item>

            <Form.Item
              label="密码"
              name="adminPassword"
              rules={[
                { required: true, message: '请输入密码' },
                { min: 6, message: '密码长度至少为 6 位' },
              ]}
            >
              <Input.Password prefix={<LockOutlined style={{ color: 'rgba(255,255,255,0.3)' }} />} placeholder="至少 6 位" size="large" />
            </Form.Item>

            <Form.Item
              label="确认密码"
              name="adminConfirmPassword"
              rules={[
                { required: true, message: '请确认密码' },
                ({ getFieldValue }) => ({
                  validator(_, value) {
                    if (!value || getFieldValue('adminPassword') === value) {
                      return Promise.resolve();
                    }
                    return Promise.reject(new Error('两次密码输入不一致'));
                  },
                }),
              ]}
            >
              <Input.Password prefix={<LockOutlined style={{ color: 'rgba(255,255,255,0.3)' }} />} placeholder="再次输入密码" size="large" />
            </Form.Item>

            <div style={{ display: 'flex', gap: 8, marginTop: 8 }}>
              <Button size="large" onClick={() => setCurrent(0)}>
                <ArrowLeftOutlined /> 上一步
              </Button>
              <Button type="primary" size="large" loading={loading} onClick={onFinish} style={{ flex: 1 }}>
                完成初始化
              </Button>
            </div>
          </Form>
        )}

        {/* Step 2: 完成 */}
        {current === 2 && (
          <div style={{ textAlign: 'center', padding: '32px 0' }}>
            <div style={{
              width: 72,
              height: 72,
              borderRadius: '50%',
              background: 'rgba(0,184,148,0.15)',
              display: 'inline-flex',
              alignItems: 'center',
              justifyContent: 'center',
              marginBottom: 20,
            }}>
              <CheckCircleOutlined style={{ fontSize: 36, color: '#00b894' }} />
            </div>
            <Title level={3} style={{ color: '#fff' }}>初始化完成</Title>
            <Paragraph style={{ color: 'rgba(255,255,255,0.45)', marginBottom: 24 }}>
              Shadow 管理面板已配置完成，请使用管理员账号登录。
            </Paragraph>
            <Button type="primary" size="large" onClick={() => navigate('/login')} style={{ minWidth: 200 }}>
              前往登录
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
