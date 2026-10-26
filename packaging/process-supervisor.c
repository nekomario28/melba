/* Host-side lifetime owner, not part of the Apple SDK ABI. No credential IPC.
 * argv: expected owner TGID (0 = explicitly detached CLI start), program, args.
 *
 * This process leads the child's process group and the worker stays in it, so
 * the owner only ever signals a PID it still holds unreaped. Cleanup here ends
 * the worker and every descendant by kernel-exact child list, never by a port
 * or /proc search for unrelated processes.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/prctl.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static volatile sig_atomic_t stopping;
static void stop(int signal_number) {
  (void)signal_number;
  stopping = 1;
}

/* Kernel-exact list of this supervisor's own children (it is a subreaper, so
 * adopted descendants appear here too) from /proc/thread-self/children.
 * Returns how many PIDs were written, or -1 when the list is unavailable. */
static int own_children(pid_t *out, int capacity) {
  int descriptor = open("/proc/thread-self/children", O_RDONLY | O_CLOEXEC);
  if (descriptor < 0)
    return -1;
  char buffer[4096];
  ssize_t count = read(descriptor, buffer, sizeof buffer - 1);
  close(descriptor);
  if (count < 0)
    return -1;
  buffer[count] = '\0';
  int found = 0;
  char *cursor = buffer;
  while (*cursor && found < capacity) {
    char *following;
    long value = strtol(cursor, &following, 10);
    if (following == cursor)
      break;
    if (value > 1 && value <= INT_MAX)
      out[found++] = (pid_t)value;
    cursor = following;
  }
  return found;
}

/* A descendant may leave the worker's process group (setsid/new group), where
 * no group signal can reach it - and it would keep the inherited session lock
 * while this loop waited for it. */
static void end_descendants(void) {
  int quiet = 0;
  /* Bounded to one second: the owner waits longer before escalating, so its
   * escalation never cuts a slow sweep short. */
  for (int round = 0; round < 200; round++) {
    pid_t children[64];
    int found = own_children(children, 64);
    if (found < 0) {
      /* No kernel list to work from. The owner holds our PID unreaped and
       * reaps us, and killing our own group leaves no descendant running. */
      kill(0, SIGKILL);
      _exit(126);
    }
    if (found == 0) {
      if (++quiet >= 3)
        return;
    } else {
      quiet = 0;
      for (int index = 0; index < found; index++)
        kill(children[index], SIGKILL);
    }
    int status;
    while (waitpid(-1, &status, WNOHANG) > 0)
      ;
    struct timespec delay = {0, 5000000};
    nanosleep(&delay, 0);
  }
}

int main(int argc, char **argv) {
  if (argc < 3)
    return 126;
  char *end;
  long owner = strtol(argv[1], &end, 10);
  if (*end || owner < 0 || owner > INT_MAX)
    return 126;
  if (owner && getppid() != (pid_t)owner)
    return 126;
  if (prctl(PR_SET_CHILD_SUBREAPER, 1L, 0L, 0L, 0L) != 0)
    return 126;
  struct sigaction action = {0};
  action.sa_handler = stop;
  sigemptyset(&action.sa_mask);
  if (sigaction(SIGTERM, &action, 0) || sigaction(SIGINT, &action, 0) ||
      sigaction(SIGHUP, &action, 0))
    return 126;
  action.sa_handler = SIG_DFL;
  if (sigaction(SIGCHLD, &action, 0))
    return 126;
  sigset_t unblocked;
  sigemptyset(&unblocked);
  sigaddset(&unblocked, SIGTERM);
  sigaddset(&unblocked, SIGINT);
  sigaddset(&unblocked, SIGHUP);
  if (sigprocmask(SIG_UNBLOCK, &unblocked, 0))
    return 126;
  pid_t child = fork();
  if (child < 0)
    return 126;
  if (child == 0) {
    action.sa_handler = SIG_DFL;
    sigaction(SIGTERM, &action, 0);
    sigaction(SIGINT, &action, 0);
    sigaction(SIGHUP, &action, 0);
    execv(argv[2], &argv[2]);
    _exit(126);
  }
  for (;;) {
    siginfo_t info = {0};
    if (stopping || (owner && getppid() != (pid_t)owner))
      break;
    if (waitid(P_PID, (id_t)child, &info, WEXITED | WNOHANG | WNOWAIT) != 0) {
      if (errno == EINTR)
        continue;
      break;
    }
    if (info.si_pid)
      break;
    struct timespec delay = {0, 50000000};
    nanosleep(&delay, 0);
  }
  /* End the worker leader and everything it left behind - including
   * descendants that escaped its process group - before reaping, because the
   * worker may be a long-running service that would otherwise block here.
   * The exit code is therefore best-effort: it reports the worker's own exit
   * only when that happened before the owner asked us to stop. */
  end_descendants();
  int status = 0;
  pid_t reaped;
  do {
    reaped = waitpid(child, &status, 0);
  } while (reaped < 0 && errno == EINTR);
  if (reaped != child)
    status = 126 << 8;
  return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}
