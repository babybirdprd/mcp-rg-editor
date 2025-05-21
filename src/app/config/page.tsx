"use client";

import { useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event"; // For potential future use with terminal output

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
import { useToast } from "@/components/ui/use-toast";
import { Toaster } from "@/components/ui/toaster";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Terminal } from "lucide-react";


// Matches the Rust Config struct (subset for UI interaction)
interface AppConfig {
  files_root: string;
  allowed_directories: string[]; // Will be joined/split for textarea
  blocked_commands: string[]; // Will be joined/split for textarea
  default_shell?: string | null;
  log_level: string;
  file_read_line_limit: number;
  file_write_line_limit: number;
  audit_log_file: string;
  fuzzy_search_log_file: string;
  mcp_log_dir: string;
  // transport_mode, sse_host, sse_port are omitted as they are less relevant for a Tauri app UI config
}

// For editable fields
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
  const { toast } = useToast();

  const fetchConfig = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const result = await invoke<AppConfig>("get_config_command");
      setConfig(result);
      setEditableConfig({
        allowed_directories_str: result.allowed_directories.join(", "),
        blocked_commands_str: result.blocked_commands.join(", "),
        default_shell_str: result.default_shell || "",
        log_level: result.log_level,
        file_read_line_limit_str: result.file_read_line_limit.toString(),
        file_write_line_limit_str: result.file_write_line_limit.toString(),
      });
    } catch (err) {
      console.error("Failed to fetch config:", err);
      setError(
        err instanceof Error ? err.message : "An unknown error occurred while fetching config.",
      );
      toast({
        variant: "destructive",
        title: "Error Fetching Config",
        description: err instanceof Error ? err.message : "Could not load configuration from backend.",
      });
    } finally {
      setIsLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    fetchConfig();
  }, [fetchConfig]);

  const handleInputChange = (
    e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>,
  ) => {
    const { name, value } = e.target;
    setEditableConfig((prev) => ({ ...prev, [name]: value }));
  };

  const handleSelectChange = (name: string, value: string) => {
    setEditableConfig((prev) => ({ ...prev, [name]: value }));
  };

  const handleSaveSetting = async (key: string, value: any) => {
    try {
      const result = await invoke<string>("set_config_value_command", {
        payload: { key, value },
      });
      toast({
        title: "Setting Saved",
        description: result || `Successfully updated ${key}.`,
      });
      await fetchConfig(); // Re-fetch to confirm and get any backend-validated values
    } catch (err) {
      console.error(`Failed to save setting ${key}:`, err);
      toast({
        variant: "destructive",
        title: `Error Saving ${key}`,
        description: err instanceof Error ? err.message : "An unknown error occurred.",
      });
    }
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center min-h-screen">
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
              You might need to set the `FILES_ROOT` environment variable.
            </AlertDescription>
          </Alert>
          <Button onClick={fetchConfig} className="mt-4">Retry</Button>
       </div>
    );
  }
  
  if (!config) {
     return (
      <div className="flex items-center justify-center min-h-screen">
        No configuration data available.
      </div>
    );
  }


  return (
    <TooltipProvider>
      <div className="container mx-auto p-4 md:p-8 space-y-6">
        <Toaster />
        <header className="mb-8">
          <h1 className="text-3xl font-bold">Application Configuration</h1>
          <p className="text-muted-foreground">
            View and manage runtime settings for the application. Some critical settings are read-only.
          </p>
        </header>

        {/* Read-only Settings */}
        <Card>
          <CardHeader>
            <CardTitle>Core Settings (Read-only)</CardTitle>
            <CardDescription>
              These settings are fundamental to the application's operation and are typically set via environment variables at startup.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div>
              <Label htmlFor="files_root">Files Root</Label>
              <Input id="files_root" value={config.files_root} readOnly />
               <p className="text-sm text-muted-foreground mt-1">
                The primary directory the application operates within.
              </p>
            </div>
             <div>
              <Label htmlFor="mcp_log_dir">Log Directory</Label>
              <Input id="mcp_log_dir" value={config.mcp_log_dir} readOnly />
              <p className="text-sm text-muted-foreground mt-1">
                Directory where audit and fuzzy search logs are stored.
              </p>
            </div>
            <div>
              <Label htmlFor="audit_log_file">Audit Log File</Label>
              <Input id="audit_log_file" value={config.audit_log_file} readOnly />
            </div>
            <div>
              <Label htmlFor="fuzzy_search_log_file">Fuzzy Search Log File</Label>
              <Input id="fuzzy_search_log_file" value={config.fuzzy_search_log_file} readOnly />
            </div>
          </CardContent>
        </Card>

        {/* Editable Settings */}
        <Card>
          <CardHeader>
            <CardTitle>Editable Runtime Settings</CardTitle>
            <CardDescription>
              Changes made here are applied in-memory for the current session.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-6">
            {/* Allowed Directories */}
            <div className="space-y-2">
              <Label htmlFor="allowed_directories_str">Allowed Directories</Label>
              <Tooltip>
                <TooltipTrigger className="w-full">
                  <Textarea
                    id="allowed_directories_str"
                    name="allowed_directories_str"
                    value={editableConfig.allowed_directories_str}
                    onChange={handleInputChange}
                    placeholder="e.g., /path/to/project1,~/project2,C:\Users\YourUser\Documents"
                    rows={3}
                  />
                </TooltipTrigger>
                <TooltipContent>
                  <p>Comma-separated list of absolute or tilde-expanded (~) paths.</p>
                  <p>These directories must be accessible and are where filesystem tools can operate.</p>
                  <p>An empty list defaults to FILES_ROOT. Use `/` or `C:\` for full (dangerous) access within FILES_ROOT scope.</p>
                </TooltipContent>
              </Tooltip>
              <Button
                onClick={() =>
                  handleSaveSetting(
                    "allowedDirectories",
                    editableConfig.allowed_directories_str.split(",").map(s => s.trim()).filter(s => s.length > 0)
                  )
                }
              >
                Save Allowed Directories
              </Button>
            </div>

            {/* Blocked Commands */}
            <div className="space-y-2">
              <Label htmlFor="blocked_commands_str">Blocked Commands</Label>
               <Tooltip>
                <TooltipTrigger className="w-full">
                  <Textarea
                    id="blocked_commands_str"
                    name="blocked_commands_str"
                    value={editableConfig.blocked_commands_str}
                    onChange={handleInputChange}
                    placeholder="e.g., rm,sudo,dd"
                    rows={3}
                  />
                </TooltipTrigger>
                <TooltipContent>
                  <p>Comma-separated list of command names (first word of command) to block from execution.</p>
                </TooltipContent>
              </Tooltip>
              <Button
                onClick={() =>
                  handleSaveSetting(
                    "blockedCommands",
                     editableConfig.blocked_commands_str.split(",").map(s => s.trim()).filter(s => s.length > 0)
                  )
                }
              >
                Save Blocked Commands
              </Button>
            </div>

            {/* Default Shell */}
            <div className="space-y-2">
              <Label htmlFor="default_shell_str">Default Shell</Label>
              <Tooltip>
                <TooltipTrigger className="w-full">
                  <Input
                    id="default_shell_str"
                    name="default_shell_str"
                    value={editableConfig.default_shell_str}
                    onChange={handleInputChange}
                    placeholder="e.g., bash, powershell (leave empty for system default)"
                  />
                </TooltipTrigger>
                <TooltipContent>
                  <p>Shell used by `execute_command` if not specified in the call. System default if empty.</p>
                </TooltipContent>
              </Tooltip>
              <Button
                onClick={() =>
                  handleSaveSetting("defaultShell", editableConfig.default_shell_str || null) // Send null if empty
                }
              >
                Save Default Shell
              </Button>
            </div>

            {/* Log Level */}
            <div className="space-y-2">
              <Label htmlFor="log_level">Log Level</Label>
              <Select
                name="log_level"
                value={editableConfig.log_level}
                onValueChange={(value) => handleSelectChange("log_level", value)}
              >
                <SelectTrigger id="log_level">
                  <SelectValue placeholder="Select log level" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="trace">Trace</SelectItem>
                  <SelectItem value="debug">Debug</SelectItem>
                  <SelectItem value="info">Info</SelectItem>
                  <SelectItem value="warn">Warn</SelectItem>
                  <SelectItem value="error">Error</SelectItem>
                </SelectContent>
              </Select>
               <p className="text-sm text-muted-foreground">
                Note: Changing log level here might not affect all logging immediately without an app restart, especially for `tracing-subscriber`. `tauri-plugin-log` might be more dynamic.
              </p>
              <Button
                onClick={() =>
                  handleSaveSetting("logLevel", editableConfig.log_level)
                }
              >
                Save Log Level (Backend May Need Restart)
              </Button>
            </div>

            {/* File Read Line Limit */}
            <div className="space-y-2">
              <Label htmlFor="file_read_line_limit_str">File Read Line Limit</Label>
              <Input
                id="file_read_line_limit_str"
                name="file_read_line_limit_str"
                type="number"
                value={editableConfig.file_read_line_limit_str}
                onChange={handleInputChange}
              />
              <Button
                onClick={() =>
                  handleSaveSetting(
                    "fileReadLineLimit",
                    parseInt(editableConfig.file_read_line_limit_str, 10) || 1000,
                  )
                }
              >
                Save Read Limit
              </Button>
            </div>

            {/* File Write Line Limit */}
            <div className="space-y-2">
              <Label htmlFor="file_write_line_limit_str">File Write Line Limit</Label>
               <p className="text-sm text-muted-foreground">
                Maximum lines `write_file` or `edit_block` will accept per call. Chunk larger writes.
              </p>
              <Input
                id="file_write_line_limit_str"
                name="file_write_line_limit_str"
                type="number"
                value={editableConfig.file_write_line_limit_str}
                onChange={handleInputChange}
              />
              <Button
                onClick={() =>
                  handleSaveSetting(
                    "fileWriteLineLimit",
                    parseInt(editableConfig.file_write_line_limit_str, 10) || 50,
                  )
                }
              >
                Save Write Limit
              </Button>
            </div>
          </CardContent>
        </Card>
      </div>
    </TooltipProvider>
  );
}