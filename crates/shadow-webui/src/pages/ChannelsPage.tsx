/**
 * Channel 管理页面
 */
import { Card, Table, Tag, Typography } from 'antd';
import { useQuery } from '@tanstack/react-query';
import { channelsApi } from '../api/client';

const { Title } = Typography;

export default function ChannelsPage() {
  const { data: channels, isLoading } = useQuery({
    queryKey: ['channels'],
    queryFn: channelsApi.list,
  });

  const columns = [
    { title: '类型', dataIndex: 'type', key: 'type', render: (v: string) => <Tag color="purple">{v}</Tag> },
    { title: '名称', dataIndex: 'alias', key: 'alias' },
    {
      title: '状态',
      dataIndex: 'enabled',
      key: 'enabled',
      render: (v: boolean) => (v ? <Tag color="success">启用</Tag> : <Tag color="default">禁用</Tag>),
    },
    {
      title: '配置',
      dataIndex: 'config',
      key: 'config',
      render: (v: Record<string, unknown>) => JSON.stringify(v),
    },
  ];

  return (
    <div>
      <Title level={2}>Channel 管理</Title>
      <Card>
        <Table
          dataSource={channels || []}
          columns={columns}
          rowKey="alias"
          loading={isLoading}
          pagination={{ pageSize: 10 }}
        />
      </Card>
    </div>
  );
}
