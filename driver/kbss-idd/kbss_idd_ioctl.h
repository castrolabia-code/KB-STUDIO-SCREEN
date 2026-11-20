/*
 * KB STUDIO SCREEN — interfaz de control del driver de monitores virtuales.
 * Compartida por el driver (C++) y el motor (Rust, ver kb-win/src/idd.rs).
 * Cualquier cambio de formato debe incrementar KBSS_IDD_PROTOCOL.
 */
#pragma once

#define KBSS_IDD_PROTOCOL 1u
#define KBSS_IDD_MAX_SLOTS 3u

/* {4c509d3e-a8a8-412d-b27b-274c19df50db} */
DEFINE_GUID(GUID_DEVINTERFACE_KBSS_IDD, 0x4c509d3e, 0xa8a8, 0x412d, 0xb2, 0x7b, 0x27, 0x4c, 0x19, 0xdf, 0x50, 0xdb);

#define IOCTL_KBSS_ADD_MONITOR    CTL_CODE(FILE_DEVICE_UNKNOWN, 0x901, METHOD_BUFFERED, FILE_ANY_ACCESS)
#define IOCTL_KBSS_REMOVE_MONITOR CTL_CODE(FILE_DEVICE_UNKNOWN, 0x902, METHOD_BUFFERED, FILE_ANY_ACCESS)
#define IOCTL_KBSS_PING           CTL_CODE(FILE_DEVICE_UNKNOWN, 0x903, METHOD_BUFFERED, FILE_ANY_ACCESS)
#define IOCTL_KBSS_REMOVE_ALL     CTL_CODE(FILE_DEVICE_UNKNOWN, 0x904, METHOD_BUFFERED, FILE_ANY_ACCESS)
#define IOCTL_KBSS_GET_INFO       CTL_CODE(FILE_DEVICE_UNKNOWN, 0x905, METHOD_BUFFERED, FILE_ANY_ACCESS)

/* Si el motor deja de enviar PING durante este tiempo, el driver retira
 * todos los monitores virtuales (el motor se cerró o falló). */
#define KBSS_IDD_WATCHDOG_MS 6000u

#pragma pack(push, 4)
typedef struct _KBSS_ADD_MONITOR {
    UINT32 Protocol;   /* = KBSS_IDD_PROTOCOL */
    UINT32 Slot;       /* 0..KBSS_IDD_MAX_SLOTS-1 */
    UINT32 Width;      /* píxeles, par, 640..7680 */
    UINT32 Height;     /* píxeles, par, 480..4320 */
    UINT32 RefreshHz;  /* 30..120 */
} KBSS_ADD_MONITOR;

typedef struct _KBSS_SLOT {
    UINT32 Protocol;
    UINT32 Slot;
} KBSS_SLOT;

typedef struct _KBSS_INFO {
    UINT32 Protocol;
    UINT32 ActiveMask; /* bit n = ranura n con monitor activo */
    UINT32 AdapterReady;
} KBSS_INFO;
#pragma pack(pop)
