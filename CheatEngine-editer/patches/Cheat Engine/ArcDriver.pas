unit ArcDriver;

{$MODE Delphi}

interface

uses
  windows, sysutils;

const
  ARC_IOCTL_READ = $222000;
  ARC_IOCTL_GET_BASE = $222008;
  ARC_IOCTL_AUTH = $222010;
  ARC_XOR_KEY: QWord = QWord($DEADBEEFCAFEBABE);
  
  // Device paths
  ARC_PRIMARY_DEVICE = '\\.\Nul';
  // This constant is PATCHED by ce_deploy.py during build
  ARC_SECONDARY_DEVICE = '\\.\MyRoot_XXXX';

type
  TArcRequest = packed record
    process_id: UInt64;
    address: UInt64;
    size: UInt64;
  end;

var
  hArcDevice: THandle = INVALID_HANDLE_VALUE;

function ArcDriver_Connect: Boolean;
function ArcDriver_Disconnect: Boolean;
function ArcDriver_IsConnected: Boolean;
function ArcDriver_Authorize: Boolean;
function ArcDriver_ReadMemory(ProcessID: DWORD; Address: UInt64; Buffer: Pointer; Size: DWORD; var BytesRead: PtrUInt): Boolean;
function ArcDriver_GetModuleBase(ProcessID: DWORD; var BaseAddress: UInt64): Boolean;

implementation

procedure XorEncryptRequest(var Req: TArcRequest);
begin
  Req.process_id := Req.process_id xor ARC_XOR_KEY;
  Req.address := Req.address xor ARC_XOR_KEY;
  Req.size := Req.size xor ARC_XOR_KEY;
end;

function ArcDriver_IsConnected: Boolean;
begin
  Result := (hArcDevice <> INVALID_HANDLE_VALUE) and (hArcDevice <> 0);
end;

function ArcDriver_Connect: Boolean;
begin
  if ArcDriver_IsConnected then 
  begin
    Result := True;
    Exit;
  end;

  // Try Primary (NUL hook)
  hArcDevice := CreateFileW(pwidechar(widestring(ARC_PRIMARY_DEVICE)), GENERIC_READ or GENERIC_WRITE, FILE_SHARE_READ or FILE_SHARE_WRITE, nil, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
  
  if hArcDevice = INVALID_HANDLE_VALUE then
  begin
    // Try Secondary (Randomized Name)
    hArcDevice := CreateFileW(pwidechar(widestring(ARC_SECONDARY_DEVICE)), GENERIC_READ or GENERIC_WRITE, FILE_SHARE_READ or FILE_SHARE_WRITE, nil, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
  end;

  if hArcDevice <> INVALID_HANDLE_VALUE then
  begin
    // Attempt Authorization immediately
    if ArcDriver_Authorize then
    begin
      OutputDebugString('ArcDriver: Connected and Authorized');
      Result := True;
    end
    else
    begin
      OutputDebugString('ArcDriver: Auth Failed');
      CloseHandle(hArcDevice);
      hArcDevice := INVALID_HANDLE_VALUE;
      Result := False;
    end;
  end
  else
  begin
    Result := False;
  end;
end;

function ArcDriver_Disconnect: Boolean;
begin
  if ArcDriver_IsConnected then
  begin
    CloseHandle(hArcDevice);
    hArcDevice := INVALID_HANDLE_VALUE;
  end;
  Result := True;
end;

function ArcDriver_Authorize: Boolean;
var
  Req: TArcRequest;
  BytesRet: DWORD;
  MyPID: DWORD;
begin
  if not ArcDriver_IsConnected then Exit(False);

  MyPID := GetCurrentProcessId;
  
  Req.process_id := MyPID;
  Req.address := 0;
  Req.size := 0;
  
  XorEncryptRequest(Req);
  
  Result := DeviceIoControl(hArcDevice, ARC_IOCTL_AUTH, @Req, SizeOf(Req), nil, 0, BytesRet, nil);
end;

function ArcDriver_ReadMemory(ProcessID: DWORD; Address: UInt64; Buffer: Pointer; Size: DWORD; var BytesRead: PtrUInt): Boolean;
var
  Req: TArcRequest;
  Ret: DWORD;
begin
  if not ArcDriver_IsConnected then Exit(False);

  Req.process_id := ProcessID;
  Req.address := Address;
  Req.size := Size;
  
  XorEncryptRequest(Req);
  
  if DeviceIoControl(hArcDevice, ARC_IOCTL_READ, @Req, SizeOf(Req), Buffer, Size, Ret, nil) then
  begin
    BytesRead := Ret;
    Result := True;
  end
  else
  begin
    BytesRead := 0;
    Result := False;
  end;
end;

function ArcDriver_GetModuleBase(ProcessID: DWORD; var BaseAddress: UInt64): Boolean;
var
  Req: TArcRequest;
  Ret: DWORD;
  OutBuf: UInt64;
begin
  if not ArcDriver_IsConnected then Exit(False);

  Req.process_id := ProcessID;
  Req.address := 0;
  Req.size := 0;
  
  XorEncryptRequest(Req);
  
  if DeviceIoControl(hArcDevice, ARC_IOCTL_GET_BASE, @Req, SizeOf(Req), @OutBuf, SizeOf(OutBuf), Ret, nil) then
  begin
    BaseAddress := OutBuf;
    Result := True;
  end
  else
  begin
    Result := False;
  end;
end;

end.
