/* Tiny launcher named "claude": Herdr only accepts agent.prompt when the pane's foreground
   process NAME is the detected agent (a script/python process is rejected with agent_not_ready).
   This shim is the foreground process "claude" and runs the real fake agent as a child. */
#include <unistd.h>
#include <sys/wait.h>
#include <stdlib.h>
int main(int argc, char **argv) {
  pid_t p = fork();
  if (p == 0) { execvp(getenv("FAKE_PY") ? getenv("FAKE_PY") : "python3", argv); _exit(127); }
  int st; waitpid(p, &st, 0); return WIFEXITED(st) ? WEXITSTATUS(st) : 1;
}
