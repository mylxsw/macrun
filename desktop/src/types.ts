export type Task = {
  task_id: string;
  kind: string;
  status: string;
  arguments: Record<string, any>;
  started_at: number;
  ended_at?: number;
  result?: { exit_code?: number };
  error?: { message: string };
  output_tail?: string;
  progress?: { received: number; total: number; bytes: number };
};
export type Settings = {
  server: string;
  cert: string;
  token_file: string;
  backend_config: string;
};
export type Snapshot = {
  policy: { paused: boolean; desktop_enabled: boolean };
  tasks: Task[];
  total_tasks: number;
  active_count: number;
  backends: {
    name: string;
    state: string;
    session?: string;
    command: string;
  }[];
  version: string;
  protocol: number;
  connection: {
    state: string;
    server: string;
    since: number;
    rtt_ms?: number;
    error?: string;
  };
};
export type AppState = {
  worker_running: boolean;
  snapshot: Snapshot | null;
  settings: Settings;
  data_dir: string;
  legacy_running: boolean;
  autostart: boolean;
  platform: string;
};
