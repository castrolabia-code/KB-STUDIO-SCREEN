/*
 * KB STUDIO SCREEN — driver de monitores virtuales (IddCx, UMDF 2).
 *
 * Basado en "IddSampleDriver" de Microsoft Windows-driver-samples,
 * Copyright (c) Microsoft Corporation, distribuido bajo la Microsoft Public
 * License (MS-PL), ver LICENSE-MS-PL.txt.
 *
 * Modificaciones de Castro Labs:
 *  - Sin monitores fijos: el motor (kbss-engine) agrega/quita monitores por
 *    IOCTL, uno por ranura (máx. 3), con la resolución REAL de cada Android.
 *  - EDID generado por ranura ("KBSS Pantalla N", fabricante CLB) con
 *    ContainerId estable para que Windows recuerde la disposición.
 *  - Watchdog: si el motor deja de enviar PING, se retiran los monitores
 *    (Windows devuelve las ventanas a los monitores físicos).
 *  - Sin trazas WPP; el driver no procesa ni guarda imágenes.
 */

#include "Driver.h"

using namespace std;
using namespace KbssIdd;
using namespace Microsoft::WRL;

#pragma region Helpers

static constexpr DWORD EDID_SIZE = 128;

static inline void FillSignalInfo(DISPLAYCONFIG_VIDEO_SIGNAL_INFO& Mode, DWORD Width, DWORD Height, DWORD VSync, bool bMonitorMode)
{
    Mode.totalSize.cx = Mode.activeSize.cx = Width;
    Mode.totalSize.cy = Mode.activeSize.cy = Height;
    Mode.AdditionalSignalInfo.vSyncFreqDivider = bMonitorMode ? 0 : 1;
    Mode.AdditionalSignalInfo.videoStandard = 255;
    Mode.vSyncFreq.Numerator = VSync;
    Mode.vSyncFreq.Denominator = 1;
    Mode.hSyncFreq.Numerator = VSync * Height;
    Mode.hSyncFreq.Denominator = 1;
    Mode.scanLineOrdering = DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE;
    Mode.pixelRate = ((UINT64)VSync) * ((UINT64)Width) * ((UINT64)Height);
}

static IDDCX_MONITOR_MODE MakeMonitorMode(const Mode& m, IDDCX_MONITOR_MODE_ORIGIN Origin)
{
    IDDCX_MONITOR_MODE r = {};
    r.Size = sizeof(r);
    r.Origin = Origin;
    FillSignalInfo(r.MonitorVideoSignalInfo, m.Width, m.Height, m.VSync, true);
    return r;
}

static IDDCX_TARGET_MODE MakeTargetMode(const Mode& m)
{
    IDDCX_TARGET_MODE r = {};
    r.Size = sizeof(r);
    FillSignalInfo(r.TargetVideoSignalInfo.targetVideoSignalInfo, m.Width, m.Height, m.VSync, false);
    return r;
}

/// Lista de modos: el pedido primero (preferido) y luego respaldos comunes.
static vector<Mode> ModeList(const Mode& preferred)
{
    vector<Mode> v = { preferred };
    const Mode fallbacks[] = { {1920, 1080, 60}, {1280, 800, 60}, {1280, 720, 60}, {1024, 768, 60} };
    for (const auto& f : fallbacks)
    {
        bool dup = false;
        for (const auto& m : v) dup = dup || (m.Width == f.Width && m.Height == f.Height);
        if (!dup) v.push_back(f);
    }
    return v;
}

/// EDID 1.4 mínimo con un timing detallado (CVT reduced blanking aproximado)
/// para el modo pedido y nombre de monitor "KBSS Pantalla N".
static void BuildEdid(BYTE (&e)[EDID_SIZE], UINT32 Slot, const Mode& m)
{
    ZeroMemory(e, EDID_SIZE);
    const BYTE header[] = { 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00 };
    memcpy(e, header, sizeof(header));
    e[8] = 0x0D; e[9] = 0x82;                       // PNP "CLB" (Castro Labs)
    e[10] = (BYTE)Slot; e[11] = 0x4B;               // código de producto
    e[12] = (BYTE)Slot; e[13] = 0x53; e[14] = 0x42; e[15] = 0x4B; // serie única por ranura
    e[16] = 1; e[17] = 36;                          // semana 1, año 2026
    e[18] = 1; e[19] = 4;                           // EDID 1.4
    e[20] = 0xA0;                                   // digital, 8 bits
    e[21] = 0; e[22] = 0;                           // tamaño físico no informado (escala 100 %)
    e[23] = 120;                                    // gamma 2.2
    e[24] = 0x06;                                   // sRGB + timing preferido en el 1er descriptor
    const BYTE chroma[] = { 0xEE, 0x91, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54 };
    memcpy(e + 25, chroma, sizeof(chroma));
    for (int i = 38; i < 54; i++) e[i] = 0x01;      // sin timings estándar

    // Descriptor 1: timing detallado.
    const DWORD hblank = 160, hfront = 48, hsync = 32;
    const DWORD vfront = 3, vsync = 5;
    DWORD vblank = (m.Height * 28 + 999) / 1000;
    if (vblank < 23) vblank = 23;
    const UINT64 clk = (UINT64)(m.Width + hblank) * (m.Height + vblank) * m.VSync / 10000; // unidades de 10 kHz
    BYTE* d = e + 54;
    d[0] = (BYTE)(clk & 0xFF); d[1] = (BYTE)((clk >> 8) & 0xFF);
    d[2] = (BYTE)(m.Width & 0xFF); d[3] = (BYTE)(hblank & 0xFF);
    d[4] = (BYTE)(((m.Width >> 8) & 0x0F) << 4 | ((hblank >> 8) & 0x0F));
    d[5] = (BYTE)(m.Height & 0xFF); d[6] = (BYTE)(vblank & 0xFF);
    d[7] = (BYTE)(((m.Height >> 8) & 0x0F) << 4 | ((vblank >> 8) & 0x0F));
    d[8] = (BYTE)(hfront & 0xFF); d[9] = (BYTE)(hsync & 0xFF);
    d[10] = (BYTE)(((vfront & 0x0F) << 4) | (vsync & 0x0F));
    d[11] = (BYTE)((((hfront >> 8) & 3) << 6) | (((hsync >> 8) & 3) << 4) | (((vfront >> 4) & 3) << 2) | ((vsync >> 4) & 3));
    d[17] = 0x1E;                                   // sincronismo digital separado, +H +V

    // Descriptor 2: nombre.
    BYTE* n = e + 72;
    n[3] = 0xFC;
    const char name[] = "KBSS Pantalla"; // 13 caracteres = máximo del descriptor
    for (int i = 0; i < 13; i++) n[5 + i] = (BYTE)name[i];

    // Descriptor 3: límites de rango.
    BYTE* r = e + 90;
    r[3] = 0xFD; r[5] = 24; r[6] = 144; r[7] = 15; r[8] = 255; r[9] = 60; r[10] = 0x00; r[11] = 0x0A;
    for (int i = 12; i < 18; i++) r[i] = 0x20;

    // Descriptor 4: texto con el número de ranura ("Ranura N").
    BYTE* t = e + 108;
    t[3] = 0xFE;
    const char txt[] = "Ranura ";
    int k = 5;
    for (int i = 0; txt[i]; i++) t[k++] = (BYTE)txt[i];
    t[k++] = (BYTE)('1' + Slot);
    t[k++] = 0x0A;
    while (k < 18) t[k++] = 0x20;

    e[126] = 0;
    BYTE sum = 0;
    for (DWORD i = 0; i < 127; i++) sum = (BYTE)(sum + e[i]);
    e[127] = (BYTE)(0x100 - sum);
}

/// Lee el modo preferido de un EDID generado por BuildEdid.
static bool ParseOurEdid(const BYTE* e, DWORD size, Mode& out)
{
    if (size != EDID_SIZE || e[8] != 0x0D || e[9] != 0x82) return false;
    const BYTE* d = e + 54;
    out.Width = d[2] | ((DWORD)(d[4] & 0xF0) << 4);
    out.Height = d[5] | ((DWORD)(d[7] & 0xF0) << 4);
    out.VSync = 60;
    return out.Width >= 640 && out.Height >= 480;
}

#pragma endregion

#pragma region WdfContexts

struct DeviceContextWrapper
{
    DeviceContext* pContext;
    bool Owner; // sólo el WDFDEVICE es dueño; el adaptador guarda una referencia
    void Cleanup()
    {
        if (Owner) delete pContext;
        pContext = nullptr;
    }
};

struct MonitorContextWrapper
{
    MonitorContext* pContext;
    void Cleanup()
    {
        delete pContext;
        pContext = nullptr;
    }
};

WDF_DECLARE_CONTEXT_TYPE(DeviceContextWrapper);
WDF_DECLARE_CONTEXT_TYPE(MonitorContextWrapper);

#pragma endregion

extern "C" DRIVER_INITIALIZE DriverEntry;
EVT_WDF_DRIVER_DEVICE_ADD KbssDeviceAdd;
EVT_WDF_DEVICE_D0_ENTRY KbssDeviceD0Entry;
EVT_IDD_CX_DEVICE_IO_CONTROL KbssIoDeviceControl;
EVT_IDD_CX_ADAPTER_INIT_FINISHED KbssAdapterInitFinished;
EVT_IDD_CX_ADAPTER_COMMIT_MODES KbssAdapterCommitModes;
EVT_IDD_CX_PARSE_MONITOR_DESCRIPTION KbssParseMonitorDescription;
EVT_IDD_CX_MONITOR_GET_DEFAULT_DESCRIPTION_MODES KbssMonitorGetDefaultModes;
EVT_IDD_CX_MONITOR_QUERY_TARGET_MODES KbssMonitorQueryModes;
EVT_IDD_CX_MONITOR_ASSIGN_SWAPCHAIN KbssMonitorAssignSwapChain;
EVT_IDD_CX_MONITOR_UNASSIGN_SWAPCHAIN KbssMonitorUnassignSwapChain;

extern "C" BOOL WINAPI DllMain(_In_ HINSTANCE, _In_ UINT, _In_opt_ LPVOID)
{
    return TRUE;
}

_Use_decl_annotations_
extern "C" NTSTATUS DriverEntry(PDRIVER_OBJECT pDriverObject, PUNICODE_STRING pRegistryPath)
{
    WDF_DRIVER_CONFIG Config;
    WDF_OBJECT_ATTRIBUTES Attributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&Attributes);
    WDF_DRIVER_CONFIG_INIT(&Config, KbssDeviceAdd);
    return WdfDriverCreate(pDriverObject, pRegistryPath, &Attributes, &Config, WDF_NO_HANDLE);
}

_Use_decl_annotations_
NTSTATUS KbssDeviceAdd(WDFDRIVER Driver, PWDFDEVICE_INIT pDeviceInit)
{
    UNREFERENCED_PARAMETER(Driver);

    WDF_PNPPOWER_EVENT_CALLBACKS PnpPowerCallbacks;
    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&PnpPowerCallbacks);
    PnpPowerCallbacks.EvtDeviceD0Entry = KbssDeviceD0Entry;
    WdfDeviceInitSetPnpPowerEventCallbacks(pDeviceInit, &PnpPowerCallbacks);

    IDD_CX_CLIENT_CONFIG IddConfig;
    IDD_CX_CLIENT_CONFIG_INIT(&IddConfig);
    // IddCx redirige IoDeviceControl a su cola interna: hay que usar este callback.
    IddConfig.EvtIddCxDeviceIoControl = KbssIoDeviceControl;
    IddConfig.EvtIddCxAdapterInitFinished = KbssAdapterInitFinished;
    IddConfig.EvtIddCxParseMonitorDescription = KbssParseMonitorDescription;
    IddConfig.EvtIddCxMonitorGetDefaultDescriptionModes = KbssMonitorGetDefaultModes;
    IddConfig.EvtIddCxMonitorQueryTargetModes = KbssMonitorQueryModes;
    IddConfig.EvtIddCxAdapterCommitModes = KbssAdapterCommitModes;
    IddConfig.EvtIddCxMonitorAssignSwapChain = KbssMonitorAssignSwapChain;
    IddConfig.EvtIddCxMonitorUnassignSwapChain = KbssMonitorUnassignSwapChain;

    NTSTATUS Status = IddCxDeviceInitConfig(pDeviceInit, &IddConfig);
    if (!NT_SUCCESS(Status)) return Status;

    WDF_OBJECT_ATTRIBUTES Attr;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&Attr, DeviceContextWrapper);
    Attr.EvtCleanupCallback = [](WDFOBJECT Object) {
        auto* p = WdfObjectGet_DeviceContextWrapper(Object);
        if (p) p->Cleanup();
    };

    WDFDEVICE Device = nullptr;
    Status = WdfDeviceCreate(&pDeviceInit, &Attr, &Device);
    if (!NT_SUCCESS(Status)) return Status;

    // Interfaz para que kbss-engine (sin administrador) controle los monitores.
    // La ACL del dispositivo se define en el INF (usuarios interactivos: lectura/escritura).
    Status = WdfDeviceCreateDeviceInterface(Device, &GUID_DEVINTERFACE_KBSS_IDD, nullptr);
    if (!NT_SUCCESS(Status)) return Status;

    Status = IddCxDeviceInitialize(Device);
    if (!NT_SUCCESS(Status)) return Status;

    auto* p = WdfObjectGet_DeviceContextWrapper(Device);
    p->pContext = new DeviceContext(Device);
    p->Owner = true;
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KbssDeviceD0Entry(WDFDEVICE Device, WDF_POWER_DEVICE_STATE PreviousState)
{
    UNREFERENCED_PARAMETER(PreviousState);
    WdfObjectGet_DeviceContextWrapper(Device)->pContext->InitAdapter();
    return STATUS_SUCCESS;
}

#pragma region Direct3DDevice

Direct3DDevice::Direct3DDevice(LUID Luid) : AdapterLuid(Luid) {}

HRESULT Direct3DDevice::Init()
{
    HRESULT hr = CreateDXGIFactory2(0, IID_PPV_ARGS(&DxgiFactory));
    if (FAILED(hr)) return hr;
    hr = DxgiFactory->EnumAdapterByLuid(AdapterLuid, IID_PPV_ARGS(&Adapter));
    if (FAILED(hr)) return hr;
    return D3D11CreateDevice(Adapter.Get(), D3D_DRIVER_TYPE_UNKNOWN, nullptr, D3D11_CREATE_DEVICE_BGRA_SUPPORT, nullptr, 0,
        D3D11_SDK_VERSION, &Device, nullptr, &DeviceContext);
}

#pragma endregion

#pragma region SwapChainProcessor

SwapChainProcessor::SwapChainProcessor(IDDCX_SWAPCHAIN hSwapChain, shared_ptr<Direct3DDevice> Device, HANDLE NewFrameEvent)
    : m_hSwapChain(hSwapChain), m_Device(Device), m_hAvailableBufferEvent(NewFrameEvent)
{
    m_hTerminateEvent.Attach(CreateEvent(nullptr, FALSE, FALSE, nullptr));
    m_hThread.Attach(CreateThread(nullptr, 0, RunThread, this, 0, nullptr));
}

SwapChainProcessor::~SwapChainProcessor()
{
    SetEvent(m_hTerminateEvent.Get());
    if (m_hThread.Get()) WaitForSingleObject(m_hThread.Get(), INFINITE);
}

DWORD CALLBACK SwapChainProcessor::RunThread(LPVOID Argument)
{
    reinterpret_cast<SwapChainProcessor*>(Argument)->Run();
    return 0;
}

void SwapChainProcessor::Run()
{
    DWORD AvTask = 0;
    HANDLE AvTaskHandle = AvSetMmThreadCharacteristicsW(L"Distribution", &AvTask);
    RunCore();
    // Borrar el swap-chain al salir obliga al SO a crear uno nuevo si hace falta.
    WdfObjectDelete((WDFOBJECT)m_hSwapChain);
    m_hSwapChain = nullptr;
    AvRevertMmThreadCharacteristics(AvTaskHandle);
}

void SwapChainProcessor::RunCore()
{
    ComPtr<IDXGIDevice> DxgiDevice;
    if (FAILED(m_Device->Device.As(&DxgiDevice))) return;

    IDARG_IN_SWAPCHAINSETDEVICE SetDevice = {};
    SetDevice.pDevice = DxgiDevice.Get();
    if (FAILED(IddCxSwapChainSetDevice(m_hSwapChain, &SetDevice))) return;

    for (;;)
    {
        ComPtr<IDXGIResource> AcquiredBuffer;
        IDARG_OUT_RELEASEANDACQUIREBUFFER Buffer = {};
        HRESULT hr = IddCxSwapChainReleaseAndAcquireBuffer(m_hSwapChain, &Buffer);
        if (hr == E_PENDING)
        {
            HANDLE WaitHandles[] = { m_hAvailableBufferEvent, m_hTerminateEvent.Get() };
            DWORD WaitResult = WaitForMultipleObjects(ARRAYSIZE(WaitHandles), WaitHandles, FALSE, 16);
            if (WaitResult == WAIT_OBJECT_0 || WaitResult == WAIT_TIMEOUT) continue;
            break;
        }
        else if (SUCCEEDED(hr))
        {
            // El frame se libera enseguida: la imagen la toma el motor con Desktop Duplication.
            AcquiredBuffer.Attach(Buffer.MetaData.pSurface);
            AcquiredBuffer.Reset();
            if (FAILED(IddCxSwapChainFinishedProcessingFrame(m_hSwapChain))) break;
        }
        else
        {
            break; // swap-chain abandonado (p. ej. DXGI_ERROR_ACCESS_LOST)
        }
    }
}

#pragma endregion

#pragma region MonitorContext

MonitorContext::MonitorContext(IDDCX_MONITOR m, Mode Preferred) : Monitor(m), m_Preferred(Preferred) {}

MonitorContext::~MonitorContext()
{
    m_ProcessingThread.reset();
}

vector<Mode> MonitorContext::Modes() const
{
    return ModeList(m_Preferred);
}

void MonitorContext::AssignSwapChain(IDDCX_SWAPCHAIN SwapChain, LUID RenderAdapter, HANDLE NewFrameEvent)
{
    m_ProcessingThread.reset();
    auto Device = make_shared<Direct3DDevice>(RenderAdapter);
    if (FAILED(Device->Init()))
    {
        WdfObjectDelete(SwapChain); // el SO reintentará con un swap-chain nuevo
    }
    else
    {
        m_ProcessingThread.reset(new SwapChainProcessor(SwapChain, Device, NewFrameEvent));
    }
}

void MonitorContext::UnassignSwapChain()
{
    m_ProcessingThread.reset();
}

#pragma endregion

#pragma region DeviceContext

DeviceContext::DeviceContext(WDFDEVICE WdfDevice) : m_WdfDevice(WdfDevice)
{
    m_LastPing = GetTickCount64();
    m_hStop.Attach(CreateEvent(nullptr, TRUE, FALSE, nullptr));
    m_hWatchdog.Attach(CreateThread(nullptr, 0, WatchdogThread, this, 0, nullptr));
}

DeviceContext::~DeviceContext()
{
    SetEvent(m_hStop.Get());
    if (m_hWatchdog.Get()) WaitForSingleObject(m_hWatchdog.Get(), INFINITE);
}

void DeviceContext::InitAdapter()
{
    IDDCX_ADAPTER_CAPS Caps = {};
    Caps.Size = sizeof(Caps);
    Caps.MaxMonitorsSupported = KBSS_IDD_MAX_SLOTS;
    Caps.EndPointDiagnostics.Size = sizeof(Caps.EndPointDiagnostics);
    Caps.EndPointDiagnostics.GammaSupport = IDDCX_FEATURE_IMPLEMENTATION_NONE;
    Caps.EndPointDiagnostics.TransmissionType = IDDCX_TRANSMISSION_TYPE_WIRED_OTHER;
    Caps.EndPointDiagnostics.pEndPointFriendlyName = L"KB STUDIO SCREEN";
    Caps.EndPointDiagnostics.pEndPointManufacturerName = L"Castro Labs";
    Caps.EndPointDiagnostics.pEndPointModelName = L"KBSS Virtual Display";
    IDDCX_ENDPOINT_VERSION Version = {};
    Version.Size = sizeof(Version);
    Version.MajorVer = 0;
    Version.MinorVer = 1;
    Caps.EndPointDiagnostics.pFirmwareVersion = &Version;
    Caps.EndPointDiagnostics.pHardwareVersion = &Version;

    WDF_OBJECT_ATTRIBUTES Attr;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&Attr, DeviceContextWrapper);

    IDARG_IN_ADAPTER_INIT AdapterInit = {};
    AdapterInit.WdfDevice = m_WdfDevice;
    AdapterInit.pCaps = &Caps;
    AdapterInit.ObjectAttributes = &Attr;

    IDARG_OUT_ADAPTER_INIT AdapterInitOut;
    if (NT_SUCCESS(IddCxAdapterInitAsync(&AdapterInit, &AdapterInitOut)))
    {
        m_Adapter = AdapterInitOut.AdapterObject;
        auto* p = WdfObjectGet_DeviceContextWrapper(AdapterInitOut.AdapterObject);
        p->pContext = this;
        p->Owner = false;
    }
}

void DeviceContext::OnAdapterReady()
{
    lock_guard<mutex> g(m_Lock);
    m_AdapterReady = true;
}

NTSTATUS DeviceContext::AddMonitor(UINT32 Slot, Mode Preferred)
{
    lock_guard<mutex> g(m_Lock);
    if (!m_AdapterReady) return STATUS_DEVICE_NOT_READY;
    if (Slot >= KBSS_IDD_MAX_SLOTS) return STATUS_INVALID_PARAMETER;
    if (m_Slots[Slot] != nullptr) return STATUS_OBJECT_NAME_COLLISION;
    m_LastPing = GetTickCount64();

    BYTE* edid = new BYTE[EDID_SIZE];
    BYTE (&e)[EDID_SIZE] = *reinterpret_cast<BYTE (*)[EDID_SIZE]>(edid);
    BuildEdid(e, Slot, Preferred);

    WDF_OBJECT_ATTRIBUTES Attr;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&Attr, MonitorContextWrapper);
    Attr.EvtCleanupCallback = [](WDFOBJECT Object) {
        auto* p = WdfObjectGet_MonitorContextWrapper(Object);
        if (p) p->Cleanup();
    };

    IDDCX_MONITOR_INFO Info = {};
    Info.Size = sizeof(Info);
    Info.MonitorType = DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI;
    Info.ConnectorIndex = Slot;
    Info.MonitorDescription.Size = sizeof(Info.MonitorDescription);
    Info.MonitorDescription.Type = IDDCX_MONITOR_DESCRIPTION_TYPE_EDID;
    Info.MonitorDescription.DataSize = EDID_SIZE;
    Info.MonitorDescription.pData = edid;
    // ContainerId estable por ranura: Windows recuerda posición y escala de cada pantalla.
    Info.MonitorContainerId = GUID_DEVINTERFACE_KBSS_IDD;
    Info.MonitorContainerId.Data4[7] = (BYTE)(0xA0 + Slot);

    IDARG_IN_MONITORCREATE Create = {};
    Create.ObjectAttributes = &Attr;
    Create.pMonitorInfo = &Info;
    IDARG_OUT_MONITORCREATE CreateOut;
    NTSTATUS Status = IddCxMonitorCreate(m_Adapter, &Create, &CreateOut);
    delete[] edid; // IddCx copia la descripción
    if (!NT_SUCCESS(Status)) return Status;

    auto* p = WdfObjectGet_MonitorContextWrapper(CreateOut.MonitorObject);
    p->pContext = new MonitorContext(CreateOut.MonitorObject, Preferred);

    IDARG_OUT_MONITORARRIVAL ArrivalOut;
    Status = IddCxMonitorArrival(CreateOut.MonitorObject, &ArrivalOut);
    if (!NT_SUCCESS(Status))
    {
        WdfObjectDelete(CreateOut.MonitorObject);
        return Status;
    }
    m_Slots[Slot] = CreateOut.MonitorObject;
    return STATUS_SUCCESS;
}

NTSTATUS DeviceContext::RemoveMonitor(UINT32 Slot)
{
    lock_guard<mutex> g(m_Lock);
    if (Slot >= KBSS_IDD_MAX_SLOTS) return STATUS_INVALID_PARAMETER;
    IDDCX_MONITOR m = m_Slots[Slot];
    if (m == nullptr) return STATUS_NOT_FOUND;
    m_Slots[Slot] = nullptr;
    return IddCxMonitorDeparture(m);
}

void DeviceContext::RemoveAll()
{
    lock_guard<mutex> g(m_Lock);
    for (UINT32 i = 0; i < KBSS_IDD_MAX_SLOTS; i++)
    {
        if (m_Slots[i] != nullptr)
        {
            IddCxMonitorDeparture(m_Slots[i]);
            m_Slots[i] = nullptr;
        }
    }
}

void DeviceContext::Ping()
{
    lock_guard<mutex> g(m_Lock);
    m_LastPing = GetTickCount64();
}

KBSS_INFO DeviceContext::Info()
{
    lock_guard<mutex> g(m_Lock);
    KBSS_INFO i = {};
    i.Protocol = KBSS_IDD_PROTOCOL;
    for (UINT32 s = 0; s < KBSS_IDD_MAX_SLOTS; s++)
        if (m_Slots[s] != nullptr) i.ActiveMask |= (1u << s);
    i.AdapterReady = m_AdapterReady ? 1 : 0;
    return i;
}

DWORD CALLBACK DeviceContext::WatchdogThread(LPVOID Argument)
{
    reinterpret_cast<DeviceContext*>(Argument)->Watchdog();
    return 0;
}

void DeviceContext::Watchdog()
{
    while (WaitForSingleObject(m_hStop.Get(), 1000) == WAIT_TIMEOUT)
    {
        bool expired = false;
        {
            lock_guard<mutex> g(m_Lock);
            bool any = false;
            for (auto m : m_Slots) any = any || (m != nullptr);
            expired = any && (GetTickCount64() - m_LastPing > KBSS_IDD_WATCHDOG_MS);
        }
        if (expired) RemoveAll();
    }
}

#pragma endregion

#pragma region DDI Callbacks

_Use_decl_annotations_
VOID KbssIoDeviceControl(WDFDEVICE Device, WDFREQUEST Request, size_t OutputBufferLength, size_t InputBufferLength, ULONG IoControlCode)
{
    UNREFERENCED_PARAMETER(OutputBufferLength);
    UNREFERENCED_PARAMETER(InputBufferLength);
    DeviceContext* ctx = WdfObjectGet_DeviceContextWrapper(Device)->pContext;
    NTSTATUS Status = STATUS_INVALID_DEVICE_REQUEST;
    ULONG_PTR Info = 0;

    switch (IoControlCode)
    {
    case IOCTL_KBSS_ADD_MONITOR:
    {
        KBSS_ADD_MONITOR* in = nullptr;
        Status = WdfRequestRetrieveInputBuffer(Request, sizeof(*in), (PVOID*)&in, nullptr);
        if (!NT_SUCCESS(Status)) break;
        // Validación estricta: la entrada viene de un proceso de usuario.
        if (in->Protocol != KBSS_IDD_PROTOCOL || in->Slot >= KBSS_IDD_MAX_SLOTS ||
            in->Width < 640 || in->Width > 4095 || in->Height < 480 || in->Height > 4095 ||
            (in->Width & 1) || (in->Height & 1) || in->RefreshHz < 30 || in->RefreshHz > 120)
        {
            Status = STATUS_INVALID_PARAMETER;
            break;
        }
        Status = ctx->AddMonitor(in->Slot, Mode{ in->Width, in->Height, in->RefreshHz });
        break;
    }
    case IOCTL_KBSS_REMOVE_MONITOR:
    {
        KBSS_SLOT* in = nullptr;
        Status = WdfRequestRetrieveInputBuffer(Request, sizeof(*in), (PVOID*)&in, nullptr);
        if (!NT_SUCCESS(Status)) break;
        Status = (in->Protocol == KBSS_IDD_PROTOCOL) ? ctx->RemoveMonitor(in->Slot) : STATUS_INVALID_PARAMETER;
        break;
    }
    case IOCTL_KBSS_REMOVE_ALL:
        ctx->RemoveAll();
        Status = STATUS_SUCCESS;
        break;
    case IOCTL_KBSS_PING:
        ctx->Ping();
        Status = STATUS_SUCCESS;
        break;
    case IOCTL_KBSS_GET_INFO:
    {
        KBSS_INFO* out = nullptr;
        Status = WdfRequestRetrieveOutputBuffer(Request, sizeof(*out), (PVOID*)&out, nullptr);
        if (!NT_SUCCESS(Status)) break;
        *out = ctx->Info();
        Info = sizeof(*out);
        break;
    }
    }
    WdfRequestCompleteWithInformation(Request, Status, Info);
}

_Use_decl_annotations_
NTSTATUS KbssAdapterInitFinished(IDDCX_ADAPTER AdapterObject, const IDARG_IN_ADAPTER_INIT_FINISHED* pInArgs)
{
    // Sin monitores al inicio: aparecen sólo cuando un Android pide "Extender".
    if (NT_SUCCESS(pInArgs->AdapterInitStatus))
    {
        WdfObjectGet_DeviceContextWrapper(AdapterObject)->pContext->OnAdapterReady();
    }
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KbssAdapterCommitModes(IDDCX_ADAPTER AdapterObject, const IDARG_IN_COMMITMODES* pInArgs)
{
    UNREFERENCED_PARAMETER(AdapterObject);
    UNREFERENCED_PARAMETER(pInArgs);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KbssParseMonitorDescription(const IDARG_IN_PARSEMONITORDESCRIPTION* pInArgs, IDARG_OUT_PARSEMONITORDESCRIPTION* pOutArgs)
{
    Mode preferred = {};
    if (!ParseOurEdid((const BYTE*)pInArgs->MonitorDescription.pData, pInArgs->MonitorDescription.DataSize, preferred))
    {
        return STATUS_INVALID_PARAMETER;
    }
    auto modes = ModeList(preferred);
    pOutArgs->MonitorModeBufferOutputCount = (UINT)modes.size();
    if (pInArgs->MonitorModeBufferInputCount < modes.size())
    {
        return (pInArgs->MonitorModeBufferInputCount > 0) ? STATUS_BUFFER_TOO_SMALL : STATUS_SUCCESS;
    }
    for (size_t i = 0; i < modes.size(); i++)
    {
        pInArgs->pMonitorModes[i] = MakeMonitorMode(modes[i], IDDCX_MONITOR_MODE_ORIGIN_MONITORDESCRIPTOR);
    }
    pOutArgs->PreferredMonitorModeIdx = 0;
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KbssMonitorGetDefaultModes(IDDCX_MONITOR MonitorObject, const IDARG_IN_GETDEFAULTDESCRIPTIONMODES* pInArgs, IDARG_OUT_GETDEFAULTDESCRIPTIONMODES* pOutArgs)
{
    // Sólo se llama para monitores sin EDID; los nuestros siempre tienen EDID,
    // pero se responde igual con los modos de la ranura por robustez.
    auto* p = WdfObjectGet_MonitorContextWrapper(MonitorObject);
    vector<Mode> modes = p && p->pContext ? p->pContext->Modes() : ModeList(Mode{ 1920, 1080, 60 });
    pOutArgs->DefaultMonitorModeBufferOutputCount = (UINT)modes.size();
    if (pInArgs->DefaultMonitorModeBufferInputCount == 0) return STATUS_SUCCESS;
    if (pInArgs->DefaultMonitorModeBufferInputCount < modes.size()) return STATUS_BUFFER_TOO_SMALL;
    for (size_t i = 0; i < modes.size(); i++)
    {
        pInArgs->pDefaultMonitorModes[i] = MakeMonitorMode(modes[i], IDDCX_MONITOR_MODE_ORIGIN_DRIVER);
    }
    pOutArgs->PreferredMonitorModeIdx = 0;
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KbssMonitorQueryModes(IDDCX_MONITOR MonitorObject, const IDARG_IN_QUERYTARGETMODES* pInArgs, IDARG_OUT_QUERYTARGETMODES* pOutArgs)
{
    auto* p = WdfObjectGet_MonitorContextWrapper(MonitorObject);
    vector<Mode> modes = p && p->pContext ? p->pContext->Modes() : ModeList(Mode{ 1920, 1080, 60 });
    vector<IDDCX_TARGET_MODE> target;
    for (const auto& m : modes) target.push_back(MakeTargetMode(m));
    pOutArgs->TargetModeBufferOutputCount = (UINT)target.size();
    if (pInArgs->TargetModeBufferInputCount >= target.size())
    {
        copy(target.begin(), target.end(), pInArgs->pTargetModes);
    }
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KbssMonitorAssignSwapChain(IDDCX_MONITOR MonitorObject, const IDARG_IN_SETSWAPCHAIN* pInArgs)
{
    WdfObjectGet_MonitorContextWrapper(MonitorObject)->pContext->AssignSwapChain(pInArgs->hSwapChain, pInArgs->RenderAdapterLuid, pInArgs->hNextSurfaceAvailable);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS KbssMonitorUnassignSwapChain(IDDCX_MONITOR MonitorObject)
{
    WdfObjectGet_MonitorContextWrapper(MonitorObject)->pContext->UnassignSwapChain();
    return STATUS_SUCCESS;
}

#pragma endregion
