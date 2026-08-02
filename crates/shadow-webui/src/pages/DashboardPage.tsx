/**
 * 仪表盘页面
 */
import { Card, Row, Col, Statistic, Typography, List, Tag, Spin } from 'antd';
import {
  CheckCircleOutlined,
  ClockCircleOutlined,
  ToolOutlined,
  DatabaseOutlined,
} from '@ant-design/icons';
import { useQuery } from '@tanstack/react-query';
import { statusApi, agentsApi, toolsApi } from '../api/client';

const { Title } = Typography;

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

  if (statusLoading || agentsLoading || toolsLoading) {
    return <Spin size="large" style={{ display: 'flex', justifyContent: 'center', marginTop: 100 }} />;
  }

  return (
    <div>
      <Title level={2}>概览</Title>

      <Row gutter={16} style={{ marginBottom: 24 }}>
        <Col span={6}>
          <Card>
            <Statistic
              title="系统状态"
              value={status?.daemon_running ? '运行中' : '已停止'}
              prefix={status?.daemon_running ? <CheckCircleOutlined /> : <ClockCircleOutlined />}
              valueStyle={{ color: status?.daemon_running ? '#52c41a' : '#ff4d4f' }}
            />
          </Card>
        </Col>
        <Col span={6}>
          <Card>
            <Statistic
              title="Agent 数量"
              value={agents?.length || 0}
              prefix={<ToolOutlined />}
            />
          </Card>
        </Col>
        <Col span={6}>
          <Card>
            <Statistic
              title="工具数量"
              value={tools?.length || 0}
              prefix={<ToolOutlined />}
            />
          </Card>
        </Col>
        <Col span={6}>
          <Card>
            <Statistic
              title="版本"
              value={status?.version || '未知'}
              prefix={<DatabaseOutlined />}
            />
          </Card>
        </Col>
      </Row>

      <Row gutter={16}>
        <Col span={12}>
          <Card title="Agent 列表" style={{ marginBottom: 16 }}>
            <List
              dataSource={agents || []}
              renderItem={(agent) => (
                <List.Item>
                  <List.Item.Meta
                    title={agent.alias}
                    description={
                      <span>
                        {agent.provider && <Tag color="blue">{agent.provider}</Tag>}
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
        <Col span={12}>
          <Card title="可用工具">
            <List
              dataSource={tools || []}
              renderItem={(tool) => (
                <List.Item>
                  <List.Item.Meta
                    title={tool.name}
                    description={tool.description}
                  />
                </List.Item>
              )}
              locale={{ emptyText: '暂无工具' }}
            />
          </Card>
        </Col>
      </Row>

      <Card title="系统信息" style={{ marginTop: 16 }}>
        <p><strong>配置文件路径:</strong> {status?.config_path}</p>
        <p><strong>数据目录:</strong> {status?.data_dir}</p>
      </Card>
    </div>
  );
}
