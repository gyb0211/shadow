/**
 * 工具列表页面
 */
import { Card, Table, Tag, Typography } from 'antd';
import { useQuery } from '@tanstack/react-query';
import { toolsApi } from '../api/client';

const { Title } = Typography;

export default function ToolsPage() {
  const { data: tools, isLoading } = useQuery({
    queryKey: ['tools'],
    queryFn: toolsApi.list,
  });

  const columns = [
    { title: '工具名称', dataIndex: 'name', key: 'name', render: (v: string) => <Tag color="blue">{v}</Tag> },
    { title: '描述', dataIndex: 'description', key: 'description' },
  ];

  return (
    <div>
      <Title level={2}>工具列表</Title>
      <Card>
        <Table
          dataSource={tools || []}
          columns={columns}
          rowKey="name"
          loading={isLoading}
          pagination={{ pageSize: 20 }}
        />
      </Card>
    </div>
  );
}
