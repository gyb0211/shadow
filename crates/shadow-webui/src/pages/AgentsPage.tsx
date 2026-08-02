/**
 * Agent 管理页面
 */
import { useState } from 'react';
import { Card, Table, Button, Modal, Form, Input, message, Tag, Typography } from 'antd';
import { PlusOutlined, EditOutlined } from '@ant-design/icons';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { agentsApi, type AgentConfig } from '../api/client';
import { useAuth } from '../contexts/AuthContext';

const { Title } = Typography;

export default function AgentsPage() {
  const [modalVisible, setModalVisible] = useState(false);
  const [editingAgent, setEditingAgent] = useState<AgentConfig | null>(null);
  const [form] = Form.useForm();
  const queryClient = useQueryClient();
  const { user } = useAuth();
  const isAdmin = user?.role === 'admin';

  const { data: agents, isLoading } = useQuery({
    queryKey: ['agents'],
    queryFn: agentsApi.list,
  });

  const updateMutation = useMutation({
    mutationFn: ({ alias, data }: { alias: string; data: Partial<AgentConfig> }) =>
      agentsApi.update(alias, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['agents'] });
      message.success('更新成功');
      setModalVisible(false);
    },
    onError: () => {
      message.error('更新失败');
    },
  });

  const handleEdit = (agent: AgentConfig) => {
    setEditingAgent(agent);
    form.setFieldsValue(agent);
    setModalVisible(true);
  };

  const handleSubmit = async () => {
    if (!editingAgent) return;
    const values = form.getFieldsValue();
    updateMutation.mutate({ alias: editingAgent.alias, data: values });
  };

  const columns = [
    { title: '名称', dataIndex: 'alias', key: 'alias' },
    {
      title: 'Provider',
      dataIndex: 'provider',
      key: 'provider',
      render: (v: string) => v ? <Tag color="blue">{v}</Tag> : '-',
    },
    { title: '模型', dataIndex: 'model', key: 'model', render: (v: string) => v || '-' },
    { title: 'System Prompt', dataIndex: 'system_prompt', key: 'system_prompt', ellipsis: true },
    {
      title: '操作',
      key: 'action',
      render: (_: unknown, record: AgentConfig) => (
        <Button icon={<EditOutlined />} onClick={() => handleEdit(record)} disabled={!isAdmin} />
      ),
    },
  ];

  return (
    <div>
      <Title level={2}>Agent 管理</Title>

      <Card
        extra={
          isAdmin && (
            <Button type="primary" icon={<PlusOutlined />}>
              新建 Agent
            </Button>
          )
        }
      >
        <Table
          dataSource={agents || []}
          columns={columns}
          rowKey="alias"
          loading={isLoading}
          pagination={{ pageSize: 10 }}
        />
      </Card>

      <Modal
        title={`编辑 Agent: ${editingAgent?.alias}`}
        open={modalVisible}
        onOk={handleSubmit}
        onCancel={() => setModalVisible(false)}
        confirmLoading={updateMutation.isPending}
      >
        <Form form={form} layout="vertical" style={{ marginTop: 16 }}>
          <Form.Item label="Provider" name="provider">
            <Input />
          </Form.Item>
          <Form.Item label="模型" name="model">
            <Input />
          </Form.Item>
          <Form.Item label="System Prompt" name="system_prompt">
            <Input.TextArea rows={4} />
          </Form.Item>
          <Form.Item label="Max Tokens" name="max_tokens">
            <Input type="number" />
          </Form.Item>
          <Form.Item label="Temperature" name="temperature">
            <Input type="number" step="0.1" min="0" max="2" />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
