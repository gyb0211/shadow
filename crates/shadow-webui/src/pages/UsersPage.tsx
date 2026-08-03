/**
 * 用户管理页面 (admin only)
 */
import { useState } from 'react';
import { Button, Modal, Form, Input, Select, Tag, message, Popconfirm } from 'antd';
import { PlusOutlined, DeleteOutlined } from '@ant-design/icons';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { usersApi, type UserInfo } from '../api/client';
import PageHeader from '../components/PageHeader';
import DataTableCard from '../components/DataTableCard';

export default function UsersPage() {
  const [modalVisible, setModalVisible] = useState(false);
  const [form] = Form.useForm();
  const queryClient = useQueryClient();

  const { data: users, isLoading } = useQuery({
    queryKey: ['users'],
    queryFn: usersApi.list,
  });

  const createMutation = useMutation({
    mutationFn: usersApi.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['users'] });
      message.success('用户创建成功');
      setModalVisible(false);
      form.resetFields();
    },
    onError: () => message.error('用户创建失败'),
  });

  const deleteMutation = useMutation({
    mutationFn: usersApi.delete,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['users'] });
      message.success('用户删除成功');
    },
    onError: () => message.error('用户删除失败'),
  });

  const columns = [
    {
      title: 'ID',
      dataIndex: 'id',
      key: 'id',
      width: 60,
      render: (v: number) => <span style={{ color: 'rgba(255,255,255,0.35)' }}>{v}</span>,
    },
    {
      title: '用户名',
      dataIndex: 'username',
      key: 'username',
      render: (v: string) => <span style={{ color: 'rgba(255,255,255,0.85)' }}>{v}</span>,
    },
    {
      title: '角色',
      dataIndex: 'role',
      key: 'role',
      width: 120,
      render: (v: string) =>
        v === 'admin' ? <Tag color="red">管理员</Tag> : <Tag color="cyan">查看者</Tag>,
    },
    {
      title: '操作',
      key: 'action',
      width: 60,
      render: (_: unknown, record: UserInfo) => (
        <Popconfirm
          title="确定要删除该用户吗？"
          onConfirm={() => deleteMutation.mutate(record.id)}
          okText="确定"
          cancelText="取消"
        >
          <Button type="text" danger icon={<DeleteOutlined />} size="small" />
        </Popconfirm>
      ),
    },
  ];

  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
      <PageHeader
        title="用户管理"
        subtitle="管理系统用户和权限"
        extra={
          <Button type="primary" icon={<PlusOutlined />} onClick={() => setModalVisible(true)}>
            新建用户
          </Button>
        }
      />
      <DataTableCard
        loading={isLoading}
        dataSource={users || []}
        columns={columns}
        rowKey="id"
        emptyText="暂无用户"
      />

      <Modal
        title="新建用户"
        open={modalVisible}
        onOk={() => form.validateFields().then((v) => createMutation.mutate(v))}
        onCancel={() => setModalVisible(false)}
        confirmLoading={createMutation.isPending}
      >
        <Form form={form} layout="vertical" style={{ marginTop: 16 }}>
          <Form.Item label="用户名" name="username" rules={[{ required: true, message: '请输入用户名' }]}>
            <Input />
          </Form.Item>
          <Form.Item label="密码" name="password" rules={[{ required: true, message: '请输入密码' }, { min: 6, message: '密码长度至少为 6 位' }]}>
            <Input.Password />
          </Form.Item>
          <Form.Item label="角色" name="role" rules={[{ required: true, message: '请选择角色' }]} initialValue="viewer">
            <Select>
              <Select.Option value="admin">管理员</Select.Option>
              <Select.Option value="viewer">查看者</Select.Option>
            </Select>
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
