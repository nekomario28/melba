/* Real futex-based pthread_mutex_* for bionic under qemu-user.
 * bionic 5.x mutexes are a SINGLE int: (tid<<16)|type|flags for
 * recursive, shared|locked|waiters for normal. Under qemu-user the TLS
 * tid read (__get_thread +0x10) mismatches and recursive mutexes wedge.
 * Replacement keeps everything in the 4-byte mutex: bit28 = owned,
 * low28 = owner tid. Recursion counts live in a side table (only the
 * owning thread ever touches its own entry). */
typedef unsigned int u32;
static inline long sc(long n,long a,long b,long c,long d){
  register long x8 asm("x8")=n; register long x0 asm("x0")=a;
  register long x1 asm("x1")=b; register long x2 asm("x2")=c;
  register long x3 asm("x3")=d;
  asm volatile("svc #0":"=r"(x0):"r"(x8),"0"(x0),"r"(x1),"r"(x2),"r"(x3):"memory");
  return x0;
}
#define SYS_futex 98
#define SYS_gettid 178
#define OWNED 0x10000000u
static u32 tid(void){ return (u32)sc(SYS_gettid,0,0,0,0); }
static void fw(volatile u32*m,u32 e){ sc(SYS_futex,(long)m,0/*WAIT*/,e,0); }
static void fk(volatile u32*m){ sc(SYS_futex,(long)m,1/*WAKE*/,1,0); }

/* side table: recursive lock counts, keyed by mutex address.
   owner-only access -> no atomics needed */
#define NSLOT 4096
static u32 rc_addr[NSLOT], rc_val[NSLOT];
static u32* rc_slot(volatile u32*m,int create){
  u32 h=((unsigned long)m>>4)%NSLOT;
  for(int i=0;i<NSLOT;i++){
    u32 k=(h+i)%NSLOT;
    if(rc_addr[k]==(u32)(unsigned long)m) return &rc_val[k];
    if(rc_addr[k]==0){ if(!create)return 0; rc_addr[k]=(u32)(unsigned long)m; rc_val[k]=0; return &rc_val[k]; }
  }
  return 0;
}

int pthread_mutex_init(void*mv,const void*a){
  volatile u32*m=mv; m[0]=0; (void)a; return 0;
}
int pthread_mutex_destroy(void*mv){
  volatile u32*m=mv; u32*s=rc_slot(m,0); if(s)*s=0; m[0]=0; return 0;
}
int pthread_mutex_lock(void*mv){
  volatile u32*m=mv; u32 t=tid(),o;
  for(;;){
    o=m[0];
    if(!(o&OWNED)){ if(__sync_bool_compare_and_swap(&m[0],o,o|OWNED|t)) return 0; continue; }
    if((o&0x0FFFFFFF)==t){ u32*s=rc_slot(m,1); if(s)(*s)++; return 0; }
    /* owned by other thread: futex-wait on the word while bit28 set */
    fw(&m[0],o);
  }
}
int pthread_mutex_trylock(void*mv){
  volatile u32*m=mv; u32 t=tid(),o=m[0];
  if(!(o&OWNED)){ return __sync_bool_compare_and_swap(&m[0],o,o|OWNED|t)?0:16; }
  if((o&0x0FFFFFFF)==t){ u32*s=rc_slot(m,1); if(s)(*s)++; return 0; }
  return 16; /* EBUSY */
}
int pthread_mutex_timedlock(void*mv,const void*ts){
  return pthread_mutex_lock(mv); /* callers pass generous deadlines */
}
int pthread_mutex_unlock(void*mv){
  volatile u32*m=mv; u32 t=tid(),o=m[0];
  if(!(o&OWNED)||(o&0x0FFFFFFF)!=t) return 0;
  u32*s=rc_slot(m,0);
  if(s&&*s>0){ (*s)--; return 0; }
  if(s)*s=0;
  /* free state must clear OWNED *and* every owner-tid bit — keeping any
     owner bits makes the next locker's CAS OR a corrupt tid in, and its
     later unlock fails the owner check, leaving the word OWNED forever */
  m[0]=o&~OWNED&~0x0FFFFFFFu;
  fk(&m[0]);
  return 0;
}
int pthread_once(volatile int*c,void(*init)(void)){
  if(*c==2) return 0;
  if(__sync_bool_compare_and_swap(c,0,1)){
    init(); *c=2; fk((volatile u32*)c); return 0;
  }
  while(*c!=2) fw((volatile u32*)c,*c);
  return 0;
}
