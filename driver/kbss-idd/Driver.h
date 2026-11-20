/*
 * KB STUDIO SCREEN — driver de monitores virtuales (IddCx, UMDF 2).
 *
 * Basado en "IddSampleDriver" de Microsoft Windows-driver-samples,
 * Copyright (c) Microsoft Corporation, distribuido bajo la Microsoft Public
 * License (MS-PL), ver LICENSE-MS-PL.txt. Modificaciones: Castro Labs.
 */
#pragma once

#define NOMINMAX
#include <windows.h>
#include <bugcodes.h>
#include <wudfwdm.h>
#include <wdf.h>
#include <iddcx.h>
#include <initguid.h>

#include <dxgi1_5.h>
#include <d3d11_2.h>
#include <avrt.h>
#include <wrl.h>

#include <memory>
#include <mutex>
#include <vector>

#include "kbss_idd_ioctl.h"

namespace Microsoft { namespace WRL { namespace Wrappers {
    typedef HandleT<HandleTraits::HANDLENullTraits> Thread;
} } }

namespace KbssIdd
{
    struct Mode { DWORD Width; DWORD Height; DWORD VSync; };

    struct Direct3DDevice
    {
        explicit Direct3DDevice(LUID AdapterLuid);
        HRESULT Init();

        LUID AdapterLuid;
        Microsoft::WRL::ComPtr<IDXGIFactory5> DxgiFactory;
        Microsoft::WRL::ComPtr<IDXGIAdapter1> Adapter;
        Microsoft::WRL::ComPtr<ID3D11Device> Device;
        Microsoft::WRL::ComPtr<ID3D11DeviceContext> DeviceContext;
    };

    /// Consume los frames del swap-chain del monitor virtual. El driver NO
    /// procesa la imagen: el motor la captura con Desktop Duplication como
    /// cualquier otro monitor. Así el código privilegiado se mantiene mínimo.
    class SwapChainProcessor
    {
    public:
        SwapChainProcessor(IDDCX_SWAPCHAIN hSwapChain, std::shared_ptr<Direct3DDevice> Device, HANDLE NewFrameEvent);
        ~SwapChainProcessor();

    private:
        static DWORD CALLBACK RunThread(LPVOID Argument);
        void Run();
        void RunCore();

        IDDCX_SWAPCHAIN m_hSwapChain;
        std::shared_ptr<Direct3DDevice> m_Device;
        HANDLE m_hAvailableBufferEvent;
        Microsoft::WRL::Wrappers::Thread m_hThread;
        Microsoft::WRL::Wrappers::Event m_hTerminateEvent;
    };

    class MonitorContext
    {
    public:
        MonitorContext(IDDCX_MONITOR Monitor, Mode Preferred);
        ~MonitorContext();

        void AssignSwapChain(IDDCX_SWAPCHAIN SwapChain, LUID RenderAdapter, HANDLE NewFrameEvent);
        void UnassignSwapChain();
        /// Modo pedido por el motor primero (preferido) y luego respaldos estándar.
        std::vector<Mode> Modes() const;

        IDDCX_MONITOR Monitor;

    private:
        Mode m_Preferred;
        std::unique_ptr<SwapChainProcessor> m_ProcessingThread;
    };

    class DeviceContext
    {
    public:
        explicit DeviceContext(WDFDEVICE WdfDevice);
        ~DeviceContext();

        void InitAdapter();
        void OnAdapterReady();

        NTSTATUS AddMonitor(UINT32 Slot, Mode Preferred);
        NTSTATUS RemoveMonitor(UINT32 Slot);
        void RemoveAll();
        void Ping();
        KBSS_INFO Info();

    private:
        static DWORD CALLBACK WatchdogThread(LPVOID Argument);
        void Watchdog();

        WDFDEVICE m_WdfDevice;
        IDDCX_ADAPTER m_Adapter = nullptr;
        bool m_AdapterReady = false;
        std::mutex m_Lock;
        IDDCX_MONITOR m_Slots[KBSS_IDD_MAX_SLOTS] = {};
        ULONGLONG m_LastPing = 0;
        Microsoft::WRL::Wrappers::Thread m_hWatchdog;
        Microsoft::WRL::Wrappers::Event m_hStop;
    };
}
