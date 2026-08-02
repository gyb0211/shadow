/**
 * 日志查看页面
 */
import { useState } from 'react';
import { Card, Table, Select, Button, Typography, Tag, Space } from 'antd';
import { ReloadOutlined } from '@ant-design/icons';
import { useQuery } from '@tanstack/react-query';
import { logsApi } from '../api/client';

const { Title } = Typography;

const levelColors: Record<string, string> = {
  ERROR: 'red',
  WARN: 'orange',
  INFO: 'blue',
  DEBUG: 'gray',
  TRACE: 'default',
};

export default function LogsPage() {
  const [level, setLevel] = useState<string>('INFO');
  const [limit, setLimit] = useState<number>(100);

  const { data: logs, isLoading, refetch } = useQuery({
    queryKey: ['logs', level, limit],
    queryFn: () => logsApi.list({ level, limit }),
  });

  const columns = [
    {
      title: '时间',
      dataIndex: 'timestamp',
      key: 'timestamp',
      width: 180,
    },
    {
      title: '级别',
      dataIndex: 'level',
      key: 'level',
      width: 80,
      render: (v: string) => <Tag color={levelColors[v] || 'default'}>{v}</Tag>,
    },
    {
      title: '目标',
      dataIndex: 'target',
      key: 'target',
      width: 200,
    },
    {
      title: '消息',
      dataIndex: 'message',
      key: 'message',
    },
  ];

  return (
    <div>
      <Title level={2}>日志查看</Title>

      <Card style={{ marginBottom: 16 }}>
        <Space>
          <Select value={level} onChange={setLevel} style={{ width: 120 }}>
            <Select.Option value="">全部</Select.Option>
            <Select.Option value="ERROR">ERROR</Select.Option>
            <Select.Option value="WARN">WARN</Select.Option>
            <Select.Option value="INFO">INFO</Select.Option>
            <Select.Option value="DEBUG">DEBUG</Select.Option>
          </Select>

          <Select value={limit} onChange={setLimit} style={{ width: 120 }}>
            <Select.Option value={50}>50 条</Select.Option>
            <Select.Option value={100}>100 条</Select.Option>
            <Select.Option value={200}>200 条</Select.Option>
            <Select.Option value={500}>500 条</Select.Option>
          </Select>

          <Button icon={<ReloadOutlined />} onClick={() => refetch()}>
            刷新
          </Button>
        </Space>
      </Card>

      <Card>
        <Table
          dataSource={logs || []}
          columns={columns}
          rowKey="timestamp"
          loading={isLoading}
          pagination={{ pageSize: 20, showSizeChanger: false }}
          scroll={{ x: 800 }}
        />
      </Card>
    </div>
  );
}
