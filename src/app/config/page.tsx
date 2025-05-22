// FILE: src/app/config/page.tsx
// IMPORTANT NOTE: Rewrite the entire file.
"use client";

import { useEffect, useState, useCallback, ChangeEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
// import { emit, listen } from "@tauri-apps/api/event"; // Not used in this version
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Terminal } from "lucide-react"; // Assuming lucide-react is installed

// Matches the Rust Config struct (subset for UI interaction)
interface AppConfig {
  files_root: string;
  allowed_directories: string[];
  blocked_commands: string[];
  default_shell?: string | null;
  log_level: string;
  file_read_line_limit: number;
  file_write_line_limit: number;
  audit_log_file: string;
  fuzzy_search_log_file: string;
  mcp_log_dir: string;
}

interface EditableConfig {
  allowed_directories_str: string;
  blocked_commands_str: string;
  default_shell_str: string;
  log_level: string;
  file_read_line_limit_str: string;
  file_write_line_limit_str: string;
}

export default function ConfigPage() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [editableConfig, setEditableConfig] = useState<EditableConfig>({
    allowed_directories_str: "",
    blocked_commands_str: "",
    default_shell_str: "",
    log_level: "info",
    file_read_line_limit_str: "1000",
    file_write_line_limit_str: "50",
  });
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchConfig = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const result = await invoke<AppConfig>("get_config_command");
      setConfig(result);
      setEditableConfig({
        allowed_directories_str: result.allowed_directories.join(", "),
        blocked_commands_str: result.blocked_commands.join(", "),
        default_shell_str: result.default_shell ?? "",
        log_level: result.log_level,
        file_read_line_limit_str: result.file_read_line_limit.toString(),
        file_write_line_limit_str: result.file_write_line_limit.toString(),
      });
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      console.error("Failed to fetch config:", errorMessage);
      setError(errorMessage);
      toast.error(
        `Could not load configuration: ${errorMessage}`,
        { description: "Error Fetching Config" }
      );
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void fetchConfig();
  }, [fetchConfig]);

  const handleInputChange = (
    e: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>,
  ) => {
    const { name, value } = e.target;
    setEditableConfig((prev) => ({ ...prev, [name]: value }));
  };

  const handleSelectChange = (name: string, value: string) => {
    setEditableConfig((prev) => ({ ...prev, [name]: value }));
  };

  const handleSaveSetting = async (key: string, value: unknown) => {
    try {
      const result = await invoke<string>("set_config_value_command", {
        payload: { key, value },
      });
      toast.success(result || `Successfully updated ${key}.`, {
        description: "Setting Saved",
      });
      await fetchConfig();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      console.error(`Failed to save setting ${key}:`, errorMessage);
      toast.error(errorMessage, {
        description: `Error Saving ${key}`,
      });
    }
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center min-h-screen p-4">
        Loading configuration...
      </div>
    );
  }

  if (error && !config) {
    return (
      <div className="flex flex-col items-center justify-center min-h-screen p-4">
        <Alert variant="destructive" className="max-w-md">
          <Terminal className="h-4 w-4" />
          <AlertTitle>Failed to Load Configuration</AlertTitle>
          <AlertDescription>
            {error}
            <br />
            Please check the backend logs and ensure the application is running correctly.
            You might need to set environment variables like `FILES_ROOT`.
          </AlertDescription>
        </Alert>
        <Button onClick={() => { void fetchConfig(); }} className="mt-4">Retry</Button>
      </div>
    );
  }

  if (!config) {
     return (
      <div className="flex items-center justify-center min-h-screen p-4">
        No configuration data available. Ensure the backend is running and accessible.
      </div>
    );
  }

  return (
    <TooltipProvider>
      <div className="container mx-auto p-4 md:p-8 space-y-6">
        <header className="mb-8">
          <h1 className="text-3xl font-bold">Application Configuration</h1>
          <p className="text-muted-foreground">
            View and manage runtime settings. Some critical settings are read-only from environment variables.
          </p>
        </header>

        <Card>
          <CardHeader>
            <CardTitle>Core Settings (Read-only)</CardTitle>
            <CardDescription>
              Fundamental settings typically set via environment variables at startup.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div>
              <Label htmlFor="files_root">Files Root</Label>
              <Input id="files_root" value={config.files_root} readOnly />
               <p className="text-sm text-muted-foreground mt-1">
                The primary directory the application operates within. (Env: FILES_ROOT)
              </p>
            </div>
             <div>
              <Label htmlFor="mcp_log_dir">Log Directory</Label>
              <Input id="mcp_log_dir" value={config.mcp_log_dir} readOnly />
              <p className="text-sm text-muted-foreground mt-1">
                Directory for audit and fuzzy search logs. (Env: MCP_LOG_DIR or derived)
              </p>
            </div>
            <div><Label htmlFor="audit_log_file">Audit Log File Path</Label><Input id="audit_log_file" value={config.audit_log_file} readOnly /></div>
            <div><Label htmlFor="fuzzy_search_log_file">Fuzzy Search Log File Path</Label><Input id="fuzzy_search_log_file" value={config.fuzzy_search_log_file} readOnly /></div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Editable Runtime Settings</CardTitle>
            <CardDescription>
              Changes are applied in-memory for the current session.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-6">
            <div className="space-y-2">
              <Label htmlFor="allowed_directories_str">Allowed Directories (Env: ALLOWED_DIRECTORIES)</Label>
              <Tooltip>
                <TooltipTrigger className="w-full">
                  <Textarea id="allowed_directories_str" name="allowed_directories_str" value={editableConfig.allowed_directories_str} onChange={handleInputChange} placeholder="e.g., /path/project1,~/project2" rows={3}/>
                </TooltipTrigger>
                <TooltipContent><p>Comma-separated list of absolute or tilde-expanded paths. Empty defaults to FILES_ROOT.</p></TooltipContent>
              </Tooltip>
              <Button onClick={() => { void handleSaveSetting("allowedDirectories", editableConfig.allowed_directories_str.split(",").map(s => s.trim()).filter(s => s)); }}>Save Allowed Dirs</Button>
            </div>

            <div className="space-y-2">
              <Label htmlFor="blocked_commands_str">Blocked Commands (Env: BLOCKED_COMMANDS)</Label>
               <Tooltip>
                <TooltipTrigger className="w-full">
                  <Textarea id="blocked_commands_str" name="blocked_commands_str" value={editableConfig.blocked_commands_str} onChange={handleInputChange} placeholder="e.g., rm,sudo" rows={3}/>
                </TooltipTrigger>
                <TooltipContent><p>Comma-separated list of command names to block.</p></TooltipContent>
              </Tooltip>
              <Button onClick={() => { void handleSaveSetting("blockedCommands", editableConfig.blocked_commands_str.split(",").map(s => s.trim()).filter(s => s)); }}>Save Blocked Cmds</Button>
            </div>

            <div className="space-y-2">
              <Label htmlFor="default_shell_str">Default Shell (Env: DEFAULT_SHELL)</Label>
              <Tooltip>
                <TooltipTrigger className="w-full">
                  <Input id="default_shell_str" name="default_shell_str" value={editableConfig.default_shell_str} onChange={handleInputChange} placeholder="e.g., bash (empty for system default)"/>
                </TooltipTrigger>
                <TooltipContent><p>Shell for `execute_command`. System default if empty.</p></TooltipContent>
              </Tooltip>
              <Button onClick={() => { void handleSaveSetting("defaultShell", editableConfig.default_shell_str || null); }}>Save Default Shell</Button>
            </div>

            <div className="space-y-2">
              <Label htmlFor="log_level">Log Level (Env: LOG_LEVEL)</Label>
              <Select
                name="log_level"
                value={editableConfig.log_level}
                onValueChange={(value: string) => { handleSelectChange("log_level", value); }}
              >
                <SelectTrigger id="log_level"><SelectValue placeholder="Select log level" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="trace">Trace</SelectItem>
                  <SelectItem value="debug">Debug</SelectItem>
                  <SelectItem value="info">Info</SelectItem>
                  <SelectItem value="warn">Warn</SelectItem>
                  <SelectItem value="error">Error</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-sm text-muted-foreground">Backend restart may be needed for full effect.</p>
              <Button onClick={() => { void handleSaveSetting("logLevel", editableConfig.log_level); }}>Save Log Level</Button>
            </div>

            <div className="space-y-2">
              <Label htmlFor="file_read_line_limit_str">File Read Line Limit (Env: FILE_READ_LINE_LIMIT)</Label>
              <Input id="file_read_line_limit_str" name="file_read_line_limit_str" type="number" value={editableConfig.file_read_line_limit_str} onChange={handleInputChange}/>
              <Button onClick={() => { void handleSaveSetting("fileReadLineLimit", parseInt(editableConfig.file_read_line_limit_str, 10) || 1000); }}>Save Read Limit</Button>
            </div>

            <div className="space-y-2">
              <Label htmlFor="file_write_line_limit_str">File Write Line Limit (Env: FILE_WRITE_LINE_LIMIT)</Label>
              <p className="text-sm text-muted-foreground">Max lines for `write_file`/`edit_block` per call.</p>
              <Input id="file_write_line_limit_str" name="file_write_line_limit_str" type="number" value={editableConfig.file_write_line_limit_str} onChange={handleInputChange}/>
              <Button onClick={() => { void handleSaveSetting("fileWriteLineLimit", parseInt(editableConfig.file_write_line_limit_str, 10) || 50); }}>Save Write Limit</Button>
            </div>
          </CardContent>
        </Card>
      </div>
    </TooltipProvider>
  );
}