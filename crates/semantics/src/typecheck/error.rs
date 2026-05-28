use frontend::span::Span;

pub use frontend::parse::Output;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TcError {
    // 3000-3099: Internal (catch-all)
    Internal { span: Span },

    // 3200-3299: Stack/Control Flow/Argument/Binding Errors
    NoSig { span: Span },
    ExpectedIdent { span: Span },
    StackUnderflow { span: Span },
    TypeParseFailed { span: Span },
    BindingAlreadyDefined { span: Span },
    BindingCapacityExceeded { span: Span },
    StackOverflow { span: Span },
    WordNotFound { span: Span },
    SigStackUnderflow { span: Span },
    SigTypeMismatch { span: Span },
    OutputCountMismatch { span: Span },
    OutputTypeMismatch { span: Span },
    ReturnStackDepth { span: Span },
    ReturnTypeMismatch { span: Span },
    IfPopElse { span: Span },
    IfPopThen { span: Span },
    IfPopCond { span: Span },
    IfCondNotBool { span: Span },
    IfThenNotQuot { span: Span },
    IfElseNotQuot { span: Span },
    IfBranchDepth { span: Span },
    IfBranchContent { span: Span },
    WhilePopBody { span: Span },
    WhilePopCond { span: Span },
    WhileBodyNotQuot { span: Span },
    WhileCondNotQuot { span: Span },
    WhileCondDepth { span: Span },
    WhileCondNotBool { span: Span },
    WhileCondModifiedStack { span: Span },
    WhileBodyDepth { span: Span },
    WhileBodyModifiedStack { span: Span },
    LoopPopBody { span: Span },
    LoopBodyNotQuot { span: Span },
    LoopBodyDepth { span: Span },
    LoopBodyModifiedStack { span: Span },
    LockPopBody { span: Span },
    LockBodyNotQuot { span: Span },
    LockBodyDepth { span: Span },
    LockBodyModifiedStack { span: Span },
    BindNotAllowed { span: Span },
    CastParseFailed { span: Span },
    CastPopValue { span: Span },
    CastSubtypeMismatch { span: Span },
    BitcastOnSubtype { span: Span },
    BitcastWidthUnknown { span: Span },
    BitcastWidthMismatch { span: Span },
    CastNotAllowed { span: Span },
    ContractDepth { span: Span },
    ContractNotBool { span: Span },
    ContractModifiedInputs { span: Span },

    // 3500-3519: Scoped values
    PlaceParseFailed { span: Span },
    MutRefToLocal { span: Span },
    ScopedLiveAtSuspend { span: Span },
    SuspendingInNonSuspendingContext { span: Span },
    ScopedLeak { span: Span },
    EmptyStackForScoped { span: Span },
    ScopedMarkerLeak { span: Span },
    ReturnWithScoped { span: Span },
    ScopeDepthExceeded { span: Span },
    SliceTypeFailed { span: Span },
    LocalNotLive { span: Span },
    ScopedTypeMismatch { span: Span },
    LockNested { span: Span },
    ArrayIndexOob { span: Span },
    IndexError { span: Span },

    // 3520-3522: Resources/db
    ResourceNameInvalid { span: Span },
    ResourceTypeInvalid { span: Span },
    ResourceCapacityExceeded { span: Span },

    // 3600-3614: MMIO
    MmioPlaceTooDeep { span: Span },
    MmioMapNotFound { span: Span },
    MmioRegNotFound { span: Span },
    MmioArrayIndexOob { span: Span },
    MmioArrayIndexNonArray { span: Span },
    MmioFieldNotFound { span: Span },
    MmioFieldNotAddressable { span: Span },
    MmioAccessViolation { span: Span },
    MmioReadNotAllowed { span: Span },
    MmioRegMisaligned { span: Span },
    MmioRegUnknownWidth { span: Span },
    MmioTypedTypeMismatch { span: Span },
    MmioTypedNotAllowed { span: Span },

    // 3615-3631: MMIO parsing
    MmioParseFailed { span: Span },
    MmioExpectedIdent { span: Span },
    MmioFieldBitRange { span: Span },
    MmioExpectedType { span: Span },
    MmioExpectedAccess { span: Span },
    MmioInvalidAccess { span: Span },
    MmioUnexpectedEof { span: Span },
    MmioExpectedLowBit { span: Span },
    MmioBadLowBit { span: Span },
    MmioExpectedHighBit { span: Span },
    MmioBadHighBit { span: Span },
    MmioExpectedFieldType { span: Span },
    MmioExpectedFieldAccess { span: Span },
    MmioInvalidFieldAccess { span: Span },

    // 3632-3634: MMIO typed(load/store resolution)
    MmioTypedAtomInvalid { span: Span },
    MmioTypedPopAddr { span: Span },
    MmioTypedMismatch { span: Span },

    // 3700-3718: Struct/Enum
    DestructBorrowMix { span: Span },
    DestructExpectedIdent { span: Span },
    DestructEmpty { span: Span },
    DestructNotStruct { span: Span },
    DestructFieldCount { span: Span },
    StructNameInvalid { span: Span },
    StructFieldNameInvalid { span: Span },
    StructFieldDuplicate { span: Span },
    StructFieldTypeInvalid { span: Span },
    StructDbCapacity { span: Span },
    PlaceSegmentEmpty { span: Span },
    FieldNotFound { span: Span },
    TypedLoadStoreTypeMismatch { span: Span },
    FieldSizeError { span: Span },

    // 3720-3725: Enum
    EnumNameInvalid { span: Span },
    EnumBaseTypeInvalid { span: Span },
    EnumVariantNameInvalid { span: Span },
    EnumVariantDuplicate { span: Span },
    EnumVariantCapacityExceeded { span: Span },
    EnumVariantNotFound { span: Span },

    // 3730-3734: Channel send/recv
    ChanSendPop { span: Span },
    ChanSendType { span: Span },
    ChanSendValueMismatch { span: Span },
    ChanRecvPop { span: Span },
    ChanRecvType { span: Span },

    // 3740-3743: Iso
    IsoNameInvalid { span: Span },
    IsoCapacityExceeded { span: Span },
    IsoDupForbidden { span: Span },
    IsoDropForbidden { span: Span },

    // 3750-3760: Task/Call/Quote
    TaskRunPop { span: Span },
    TaskRunNotQuot { span: Span },
    TaskRunDepth { span: Span },
    TaskRunModified { span: Span },
    TaskSpawnPop { span: Span },
    TaskSpawnSig { span: Span },
    TaskSpawnType { span: Span },
    CallPopQuot { span: Span },
    QuoteSyntax { span: Span },

    // 3900-3907: Capacity/Internal
    AtomTooLong { span: Span },
    TypeTableFull { span: Span },
    BlockTableFull { span: Span },
    BlockNotFound { span: Span },
    OpTableFull { span: Span },
    ArenaFull { span: Span },
    TooManyTypes { span: Span },
}

impl TcError {
    pub fn code(self) -> u32 {
        match self {
            TcError::Internal { .. } => 3000,
            TcError::NoSig { .. } => 3200,
            TcError::ExpectedIdent { .. } => 3201,
            TcError::StackUnderflow { .. } => 3202,
            TcError::TypeParseFailed { .. } => 3203,
            TcError::BindingAlreadyDefined { .. } => 3204,
            TcError::BindingCapacityExceeded { .. } => 3205,
            TcError::StackOverflow { .. } => 3206,
            TcError::WordNotFound { .. } => 3210,
            TcError::SigStackUnderflow { .. } => 3211,
            TcError::SigTypeMismatch { .. } => 3212,
            TcError::OutputCountMismatch { .. } => 3220,
            TcError::OutputTypeMismatch { .. } => 3221,
            TcError::ReturnStackDepth { .. } => 3230,
            TcError::ReturnTypeMismatch { .. } => 3231,
            TcError::IfPopElse { .. } => 3240,
            TcError::IfPopThen { .. } => 3241,
            TcError::IfPopCond { .. } => 3242,
            TcError::IfCondNotBool { .. } => 3243,
            TcError::IfThenNotQuot { .. } => 3244,
            TcError::IfElseNotQuot { .. } => 3245,
            TcError::IfBranchDepth { .. } => 3246,
            TcError::IfBranchContent { .. } => 3247,
            TcError::WhilePopBody { .. } => 3250,
            TcError::WhilePopCond { .. } => 3251,
            TcError::WhileBodyNotQuot { .. } => 3252,
            TcError::WhileCondNotQuot { .. } => 3253,
            TcError::WhileCondDepth { .. } => 3254,
            TcError::WhileCondNotBool { .. } => 3255,
            TcError::WhileCondModifiedStack { .. } => 3256,
            TcError::WhileBodyDepth { .. } => 3257,
            TcError::WhileBodyModifiedStack { .. } => 3258,
            TcError::LoopPopBody { .. } => 3260,
            TcError::LoopBodyNotQuot { .. } => 3261,
            TcError::LoopBodyDepth { .. } => 3262,
            TcError::LoopBodyModifiedStack { .. } => 3263,
            TcError::LockPopBody { .. } => 3270,
            TcError::LockBodyNotQuot { .. } => 3271,
            TcError::LockBodyDepth { .. } => 3272,
            TcError::LockBodyModifiedStack { .. } => 3273,
            TcError::BindNotAllowed { .. } => 3281,
            TcError::CastParseFailed { .. } => 3295,
            TcError::CastPopValue { .. } => 3297,
            TcError::CastSubtypeMismatch { .. } => 3300,
            TcError::BitcastOnSubtype { .. } => 3302,
            TcError::BitcastWidthUnknown { .. } => 3303,
            TcError::BitcastWidthMismatch { .. } => 3304,
            TcError::CastNotAllowed { .. } => 3305,
            TcError::ContractDepth { .. } => 3310,
            TcError::ContractNotBool { .. } => 3311,
            TcError::ContractModifiedInputs { .. } => 3312,
            TcError::PlaceParseFailed { .. } => 3500,
            TcError::MutRefToLocal { .. } => 3501,
            TcError::ScopedLiveAtSuspend { .. } => 3502,
            TcError::SuspendingInNonSuspendingContext { .. } => 3503,
            TcError::ScopedLeak { .. } => 3504,
            TcError::EmptyStackForScoped { .. } => 3505,
            TcError::ScopedMarkerLeak { .. } => 3506,
            TcError::ReturnWithScoped { .. } => 3511,
            TcError::ScopeDepthExceeded { .. } => 3512,
            TcError::SliceTypeFailed { .. } => 3513,
            TcError::LocalNotLive { .. } => 3514,
            TcError::ScopedTypeMismatch { .. } => 3515,
            TcError::LockNested { .. } => 3517,
            TcError::ArrayIndexOob { .. } => 3518,
            TcError::IndexError { .. } => 3519,
            TcError::ResourceNameInvalid { .. } => 3520,
            TcError::ResourceTypeInvalid { .. } => 3521,
            TcError::ResourceCapacityExceeded { .. } => 3522,
            TcError::MmioPlaceTooDeep { .. } => 3600,
            TcError::MmioMapNotFound { .. } => 3602,
            TcError::MmioRegNotFound { .. } => 3603,
            TcError::MmioArrayIndexOob { .. } => 3604,
            TcError::MmioArrayIndexNonArray { .. } => 3606,
            TcError::MmioFieldNotFound { .. } => 3607,
            TcError::MmioFieldNotAddressable { .. } => 3608,
            TcError::MmioAccessViolation { .. } => 3609,
            TcError::MmioReadNotAllowed { .. } => 3610,
            TcError::MmioRegMisaligned { .. } => 3611,
            TcError::MmioRegUnknownWidth { .. } => 3612,
            TcError::MmioTypedTypeMismatch { .. } => 3613,
            TcError::MmioTypedNotAllowed { .. } => 3614,
            TcError::MmioParseFailed { .. } => 3615,
            TcError::MmioExpectedIdent { .. } => 3616,
            TcError::MmioFieldBitRange { .. } => 3617,
            TcError::MmioExpectedType { .. } => 3618,
            TcError::MmioExpectedAccess { .. } => 3620,
            TcError::MmioInvalidAccess { .. } => 3621,
            TcError::MmioUnexpectedEof { .. } => 3622,
            TcError::MmioExpectedLowBit { .. } => 3624,
            TcError::MmioBadLowBit { .. } => 3625,
            TcError::MmioExpectedHighBit { .. } => 3626,
            TcError::MmioBadHighBit { .. } => 3627,
            TcError::MmioExpectedFieldType { .. } => 3628,
            TcError::MmioExpectedFieldAccess { .. } => 3630,
            TcError::MmioInvalidFieldAccess { .. } => 3631,
            TcError::MmioTypedAtomInvalid { .. } => 3632,
            TcError::MmioTypedPopAddr { .. } => 3633,
            TcError::MmioTypedMismatch { .. } => 3634,
            TcError::DestructBorrowMix { .. } => 3701,
            TcError::DestructExpectedIdent { .. } => 3702,
            TcError::DestructEmpty { .. } => 3703,
            TcError::DestructNotStruct { .. } => 3704,
            TcError::DestructFieldCount { .. } => 3705,
            TcError::StructNameInvalid { .. } => 3710,
            TcError::StructFieldNameInvalid { .. } => 3711,
            TcError::StructFieldDuplicate { .. } => 3712,
            TcError::StructFieldTypeInvalid { .. } => 3713,
            TcError::StructDbCapacity { .. } => 3714,
            TcError::PlaceSegmentEmpty { .. } => 3715,
            TcError::FieldNotFound { .. } => 3716,
            TcError::TypedLoadStoreTypeMismatch { .. } => 3717,
            TcError::FieldSizeError { .. } => 3718,
            TcError::EnumNameInvalid { .. } => 3720,
            TcError::EnumBaseTypeInvalid { .. } => 3721,
            TcError::EnumVariantNameInvalid { .. } => 3722,
            TcError::EnumVariantDuplicate { .. } => 3723,
            TcError::EnumVariantCapacityExceeded { .. } => 3724,
            TcError::EnumVariantNotFound { .. } => 3725,
            TcError::ChanSendPop { .. } => 3730,
            TcError::ChanSendType { .. } => 3731,
            TcError::ChanSendValueMismatch { .. } => 3732,
            TcError::ChanRecvPop { .. } => 3733,
            TcError::ChanRecvType { .. } => 3734,
            TcError::IsoNameInvalid { .. } => 3740,
            TcError::IsoCapacityExceeded { .. } => 3741,
            TcError::IsoDupForbidden { .. } => 3742,
            TcError::IsoDropForbidden { .. } => 3743,
            TcError::TaskRunPop { .. } => 3750,
            TcError::TaskRunNotQuot { .. } => 3751,
            TcError::TaskRunDepth { .. } => 3752,
            TcError::TaskRunModified { .. } => 3753,
            TcError::TaskSpawnPop { .. } => 3754,
            TcError::TaskSpawnSig { .. } => 3756,
            TcError::TaskSpawnType { .. } => 3757,
            TcError::CallPopQuot { .. } => 3758,
            TcError::QuoteSyntax { .. } => 3760,
            TcError::AtomTooLong { .. } => 3901,
            TcError::TypeTableFull { .. } => 3902,
            TcError::BlockTableFull { .. } => 3903,
            TcError::BlockNotFound { .. } => 3904,
            TcError::OpTableFull { .. } => 3905,
            TcError::ArenaFull { .. } => 3906,
            TcError::TooManyTypes { .. } => 3907,
        }
    }

    pub fn span(self) -> Span {
        match self {
            TcError::Internal { span }
            | TcError::NoSig { span }
            | TcError::ExpectedIdent { span }
            | TcError::StackUnderflow { span }
            | TcError::TypeParseFailed { span }
            | TcError::BindingAlreadyDefined { span }
            | TcError::BindingCapacityExceeded { span }
            | TcError::StackOverflow { span }
            | TcError::WordNotFound { span }
            | TcError::SigStackUnderflow { span }
            | TcError::SigTypeMismatch { span }
            | TcError::OutputCountMismatch { span }
            | TcError::OutputTypeMismatch { span }
            | TcError::ReturnStackDepth { span }
            | TcError::ReturnTypeMismatch { span }
            | TcError::IfPopElse { span }
            | TcError::IfPopThen { span }
            | TcError::IfPopCond { span }
            | TcError::IfCondNotBool { span }
            | TcError::IfThenNotQuot { span }
            | TcError::IfElseNotQuot { span }
            | TcError::IfBranchDepth { span }
            | TcError::IfBranchContent { span }
            | TcError::WhilePopBody { span }
            | TcError::WhilePopCond { span }
            | TcError::WhileBodyNotQuot { span }
            | TcError::WhileCondNotQuot { span }
            | TcError::WhileCondDepth { span }
            | TcError::WhileCondNotBool { span }
            | TcError::WhileCondModifiedStack { span }
            | TcError::WhileBodyDepth { span }
            | TcError::WhileBodyModifiedStack { span }
            | TcError::LoopPopBody { span }
            | TcError::LoopBodyNotQuot { span }
            | TcError::LoopBodyDepth { span }
            | TcError::LoopBodyModifiedStack { span }
            | TcError::LockPopBody { span }
            | TcError::LockBodyNotQuot { span }
            | TcError::LockBodyDepth { span }
            | TcError::LockBodyModifiedStack { span }
            | TcError::BindNotAllowed { span }
            | TcError::CastParseFailed { span }
            | TcError::CastPopValue { span }
            | TcError::CastSubtypeMismatch { span }
            | TcError::BitcastOnSubtype { span }
            | TcError::BitcastWidthUnknown { span }
            | TcError::BitcastWidthMismatch { span }
            | TcError::CastNotAllowed { span }
            | TcError::ContractDepth { span }
            | TcError::ContractNotBool { span }
            | TcError::ContractModifiedInputs { span }
            | TcError::PlaceParseFailed { span }
            | TcError::MutRefToLocal { span }
            | TcError::ScopedLiveAtSuspend { span }
            | TcError::SuspendingInNonSuspendingContext { span }
            | TcError::ScopedLeak { span }
            | TcError::EmptyStackForScoped { span }
            | TcError::ScopedMarkerLeak { span }
            | TcError::ReturnWithScoped { span }
            | TcError::ScopeDepthExceeded { span }
            | TcError::SliceTypeFailed { span }
            | TcError::LocalNotLive { span }
            | TcError::ScopedTypeMismatch { span }
            | TcError::LockNested { span }
            | TcError::ArrayIndexOob { span }
            | TcError::IndexError { span }
            | TcError::ResourceNameInvalid { span }
            | TcError::ResourceTypeInvalid { span }
            | TcError::ResourceCapacityExceeded { span }
            | TcError::MmioPlaceTooDeep { span }
            | TcError::MmioMapNotFound { span }
            | TcError::MmioRegNotFound { span }
            | TcError::MmioArrayIndexOob { span }
            | TcError::MmioArrayIndexNonArray { span }
            | TcError::MmioFieldNotFound { span }
            | TcError::MmioFieldNotAddressable { span }
            | TcError::MmioAccessViolation { span }
            | TcError::MmioReadNotAllowed { span }
            | TcError::MmioRegMisaligned { span }
            | TcError::MmioRegUnknownWidth { span }
            | TcError::MmioTypedTypeMismatch { span }
            | TcError::MmioTypedNotAllowed { span }
            | TcError::MmioParseFailed { span }
            | TcError::MmioExpectedIdent { span }
            | TcError::MmioFieldBitRange { span }
            | TcError::MmioExpectedType { span }
            | TcError::MmioExpectedAccess { span }
            | TcError::MmioInvalidAccess { span }
            | TcError::MmioUnexpectedEof { span }
            | TcError::MmioExpectedLowBit { span }
            | TcError::MmioBadLowBit { span }
            | TcError::MmioExpectedHighBit { span }
            | TcError::MmioBadHighBit { span }
            | TcError::MmioExpectedFieldType { span }
            | TcError::MmioExpectedFieldAccess { span }
            | TcError::MmioInvalidFieldAccess { span }
            | TcError::MmioTypedAtomInvalid { span }
            | TcError::MmioTypedPopAddr { span }
            | TcError::MmioTypedMismatch { span }
            | TcError::DestructBorrowMix { span }
            | TcError::DestructExpectedIdent { span }
            | TcError::DestructEmpty { span }
            | TcError::DestructNotStruct { span }
            | TcError::DestructFieldCount { span }
            | TcError::StructNameInvalid { span }
            | TcError::StructFieldNameInvalid { span }
            | TcError::StructFieldDuplicate { span }
            | TcError::StructFieldTypeInvalid { span }
            | TcError::StructDbCapacity { span }
            | TcError::PlaceSegmentEmpty { span }
            | TcError::FieldNotFound { span }
            | TcError::TypedLoadStoreTypeMismatch { span }
            | TcError::FieldSizeError { span }
            | TcError::EnumNameInvalid { span }
            | TcError::EnumBaseTypeInvalid { span }
            | TcError::EnumVariantNameInvalid { span }
            | TcError::EnumVariantDuplicate { span }
            | TcError::EnumVariantCapacityExceeded { span }
            | TcError::EnumVariantNotFound { span }
            | TcError::ChanSendPop { span }
            | TcError::ChanSendType { span }
            | TcError::ChanSendValueMismatch { span }
            | TcError::ChanRecvPop { span }
            | TcError::ChanRecvType { span }
            | TcError::IsoNameInvalid { span }
            | TcError::IsoCapacityExceeded { span }
            | TcError::IsoDupForbidden { span }
            | TcError::IsoDropForbidden { span }
            | TcError::TaskRunPop { span }
            | TcError::TaskRunNotQuot { span }
            | TcError::TaskRunDepth { span }
            | TcError::TaskRunModified { span }
            | TcError::TaskSpawnPop { span }
            | TcError::TaskSpawnSig { span }
            | TcError::TaskSpawnType { span }
            | TcError::CallPopQuot { span }
            | TcError::QuoteSyntax { span }
            | TcError::AtomTooLong { span }
            | TcError::TypeTableFull { span }
            | TcError::BlockTableFull { span }
            | TcError::BlockNotFound { span }
            | TcError::OpTableFull { span }
            | TcError::ArenaFull { span }
            | TcError::TooManyTypes { span } => span,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChecksMode {
    Off,
    Contracts,
    All,
}
