/**
 * 仪表盘页面 -- 渐变统计卡片 + 等高信息面板
 */
import { Card, Row, Col, Typography, List, Tag, Skeleton } from 'antd';
import {
  CheckCircleOutlined,
  ClockCircleOutlined,
  RobotOutlined,
  ToolOutlined,
  InfoCircleOutlined,
} from '@ant-design/icons';
import { useQuery } from '@tanstack/react-query';
import { statusApi, agentsApi, toolsApi } from '../api/client';

const { Title, Text } = Typography;

export default function DashboardPage() {
  const { data: status, isLoading: statusLoading } = useQuery({
    queryKey: ['status'],
    queryFn: statusApi.get,
  });
  const { data: agents, isLoading: agentsLoading } = useQuery({
    queryKey: ['agents'],
    queryFn: agentsApi.list,
  });
  const { data: tools, isLoading: toolsLoading } = useQuery({
    queryKey: ['tools'],
    queryFn: toolsApi.list,
  });

  const loading = statusLoading || agentsLoading || toolsLoading;

  if (loading) {
    return (
      <div>
        <Skeleton.Input active style={{ width: 200, height: 32, marginBottom: 24 }} />
        <Row gutter={16}>
          {[1, 2, 3, 4].map((i) => (
            <Col span={6} key={i}>
              <Card style={{ height: 100 }}>
                <Skeleton active paragraph={{ rows: 1 }} />
              </Card>
            </Col>
          ))}
        </Row>
      </div>
    );
  }

  const stats = [
    {
      title: '系统状态',
      value: status?.daemon_running ? '运行中' : '已停止',
      icon: status?.daemon_running ? <CheckCircleOutlined /> : <ClockCircleOutlined />,
      color: status?.daemon_running ? '#00b894' : '#ff7675',
      bg: status?.daemon_running
        ? 'linear-gradient(135deg, rgba(0,184,148,0.12), rgba(0,206,201,0.04))'
        : 'linear-gradient(135deg, rgba(255,118,117,0.12), rgba(255,118,117,0.04))',
    },
    {
      title: 'Agent 数量',
      value: agents?.length || 0,
      icon: <RobotOutlined />,
      color: '#a29bfe',
      bg: 'linear-gradient(135deg, rgba(108,92,231,0.12), rgba(162,155,254,0.04))',
    },
    {
      title: '工具数量',
      value: tools?.length || 0,
      icon: <ToolOutlined />,
      color: '#fdcb6e',
      bg: 'linear-gradient(135deg, rgba(253,203,110,0.12), rgba(253,203,110,0.04))',
    },
    {
      title: '版本',
      value: status?.version || '未知',
      icon: <InfoCircleOutlined />,
      color: '#00cec9',
      bg: 'linear-gradient(135deg, rgba(0,206,201,0.12), rgba(0,206,201,0.04))',
    },
  ];

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16, flex: 1, minHeight: 0 }}>
      <Title level={4} style={{ color: 'rgba(255,255,255,0.85)', margin: 0, flexShrink: 0 }}>
        概览
      </Title>

      {/* 统计卡片行 */}
      <Row gutter={16} style={{ flexShrink: 0 }}>
        {stats.map((s, i) => (
          <Col span={6} key={i}>
            <Card
              style={{ background: s.bg, border: '1px solid rgba(255,255,255,0.06)', borderRadius: 12 }}
              styles={{ body: { padding: 18 } }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 10 }}>
                <div style={{
                  width: 36, height: 36, borderRadius: 8,
                  background: 'rgba(255,255,255,0.04)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  fontSize: 16, color: s.color,
                }}>
                  {s.icon}
                </div>
                <Text style={{ color: 'rgba(255,255,255,0.45)', fontSize: 13 }}>{s.title}</Text>
              </div>
              <div style={{ color: '#fff', fontSize: 24, fontWeight: 600 }}>{s.value}</div>
            </Card>
          </Col>
        ))}
      </Row>

      {/* 系统信息 -- 紧跟统计卡片 */}
      <Card
        style={{ borderRadius: 12, flexShrink: 0 }}
        styles={{ body: { padding: '16px 20px' } }}
      >
        <Row gutter={24}>
          <Col span={12}>
            <Text style={{ color: 'rgba(255,255,255,0.35)', fontSize: 12, display: 'block', marginBottom: 2 }}>
              配置文件路径
            </Text>
            <Text style={{ color: 'rgba(255,255,255,0.75)', fontFamily: 'monospace, monospace', fontSize: 13 }}>
              {status?.config_path || '-'}
            </Text>
          </Col>
          <Col span={12}>
            <Text style={{ color: 'rgba(255,255,255,0.35)', fontSize: 12, display: 'block', marginBottom: 2 }}>
              数据目录
            </Text>
            <Text style={{ color: 'rgba(255,255,255,0.75)', fontFamily: 'monospace, monospace', fontSize: 13 }}>
              {status?.data_dir || '-'}
            </Text>
          </Col>
        </Row>
      </Card>

      {/* 等高双栏列表 */}
      <Row gutter={16} style={{ flex: 1, minHeight: 0 }}>
        <Col span={12} style={{ display: 'flex' }}>
          <Card
            className="fill-card"
            title={<span style={{ color: 'rgba(255,255,255,0.85)' }}>Agent 列表</span>}
            style={{ borderRadius: 12 }}
          >
            <List
              dataSource={agents || []}
              renderItem={(agent) => (
                <List.Item style={{ padding: '12px 24px', borderBottom: '1px solid rgba(255,255,255,0.04)' }}>
                  <List.Item.Meta
                    title={<span style={{ color: 'rgba(255,255,255,0.85)' }}>{agent.alias}</span>}
                    description={
                      <span>
                        {agent.provider && <Tag color="purple">{agent.provider}</Tag>}
                        {agent.model && <Tag>{agent.model}</Tag>}
                      </span>
                    }
                  />
                </List.Item>
              )}
              locale={{ emptyText: '暂无 Agent' }}
            />
          </Card>
        </Col>
        <Col span={12} style={{ display: 'flex' }}>
          <Card
            className="fill-card"
            title={<span style={{ color: 'rgba(255,255,255,0.85)' }}>可用工具</span>}
            style={{ borderRadius: 12 }}
          >
            <List
              dataSource={tools || []}
              renderItem={(tool) => (
                <List.Item style={{ padding: '12px 24px', borderBottom: '1px solid rgba(255,255,255,0.04)' }}>
                  <List.Item.Meta
                    title={<span style={{ color: 'rgba(255,255,255,0.85)', fontFamily: 'monospace, monospace' }}>{tool.name}</span>}
                    description={<span style={{ color: 'rgba(255,255,255,0.35)' }}>{tool.description}</span>}
                  />
                </List.Item>
              )}
              locale={{ emptyText: '暂无工具' }}
            />
          </Card>
        </Col>
      </Row>
    </div>
  );
}
