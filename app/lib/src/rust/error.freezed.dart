// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint, type=warning, deprecated_member_use, deprecated_member_use_from_same_package
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'error.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$CoreError {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is CoreError);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'CoreError()';
}


}

/// @nodoc
class $CoreErrorCopyWith<$Res>  {
$CoreErrorCopyWith(CoreError _, $Res Function(CoreError) __);
}


/// Adds pattern-matching-related methods to [CoreError].
extension CoreErrorPatterns on CoreError {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( CoreError_Network value)?  network,TResult Function( CoreError_GitHub value)?  gitHub,TResult Function( CoreError_Integrity value)?  integrity,TResult Function( CoreError_InvalidInput value)?  invalidInput,TResult Function( CoreError_Io value)?  io,TResult Function( CoreError_DecryptFailed value)?  decryptFailed,TResult Function( CoreError_Crypto value)?  crypto,required TResult orElse(),}){
final _that = this;
switch (_that) {
case CoreError_Network() when network != null:
return network(_that);case CoreError_GitHub() when gitHub != null:
return gitHub(_that);case CoreError_Integrity() when integrity != null:
return integrity(_that);case CoreError_InvalidInput() when invalidInput != null:
return invalidInput(_that);case CoreError_Io() when io != null:
return io(_that);case CoreError_DecryptFailed() when decryptFailed != null:
return decryptFailed(_that);case CoreError_Crypto() when crypto != null:
return crypto(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( CoreError_Network value)  network,required TResult Function( CoreError_GitHub value)  gitHub,required TResult Function( CoreError_Integrity value)  integrity,required TResult Function( CoreError_InvalidInput value)  invalidInput,required TResult Function( CoreError_Io value)  io,required TResult Function( CoreError_DecryptFailed value)  decryptFailed,required TResult Function( CoreError_Crypto value)  crypto,}){
final _that = this;
switch (_that) {
case CoreError_Network():
return network(_that);case CoreError_GitHub():
return gitHub(_that);case CoreError_Integrity():
return integrity(_that);case CoreError_InvalidInput():
return invalidInput(_that);case CoreError_Io():
return io(_that);case CoreError_DecryptFailed():
return decryptFailed(_that);case CoreError_Crypto():
return crypto(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( CoreError_Network value)?  network,TResult? Function( CoreError_GitHub value)?  gitHub,TResult? Function( CoreError_Integrity value)?  integrity,TResult? Function( CoreError_InvalidInput value)?  invalidInput,TResult? Function( CoreError_Io value)?  io,TResult? Function( CoreError_DecryptFailed value)?  decryptFailed,TResult? Function( CoreError_Crypto value)?  crypto,}){
final _that = this;
switch (_that) {
case CoreError_Network() when network != null:
return network(_that);case CoreError_GitHub() when gitHub != null:
return gitHub(_that);case CoreError_Integrity() when integrity != null:
return integrity(_that);case CoreError_InvalidInput() when invalidInput != null:
return invalidInput(_that);case CoreError_Io() when io != null:
return io(_that);case CoreError_DecryptFailed() when decryptFailed != null:
return decryptFailed(_that);case CoreError_Crypto() when crypto != null:
return crypto(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String message)?  network,TResult Function( int status,  String message)?  gitHub,TResult Function( String message)?  integrity,TResult Function( String message)?  invalidInput,TResult Function( String message)?  io,TResult Function()?  decryptFailed,TResult Function()?  crypto,required TResult orElse(),}) {final _that = this;
switch (_that) {
case CoreError_Network() when network != null:
return network(_that.message);case CoreError_GitHub() when gitHub != null:
return gitHub(_that.status,_that.message);case CoreError_Integrity() when integrity != null:
return integrity(_that.message);case CoreError_InvalidInput() when invalidInput != null:
return invalidInput(_that.message);case CoreError_Io() when io != null:
return io(_that.message);case CoreError_DecryptFailed() when decryptFailed != null:
return decryptFailed();case CoreError_Crypto() when crypto != null:
return crypto();case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String message)  network,required TResult Function( int status,  String message)  gitHub,required TResult Function( String message)  integrity,required TResult Function( String message)  invalidInput,required TResult Function( String message)  io,required TResult Function()  decryptFailed,required TResult Function()  crypto,}) {final _that = this;
switch (_that) {
case CoreError_Network():
return network(_that.message);case CoreError_GitHub():
return gitHub(_that.status,_that.message);case CoreError_Integrity():
return integrity(_that.message);case CoreError_InvalidInput():
return invalidInput(_that.message);case CoreError_Io():
return io(_that.message);case CoreError_DecryptFailed():
return decryptFailed();case CoreError_Crypto():
return crypto();}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String message)?  network,TResult? Function( int status,  String message)?  gitHub,TResult? Function( String message)?  integrity,TResult? Function( String message)?  invalidInput,TResult? Function( String message)?  io,TResult? Function()?  decryptFailed,TResult? Function()?  crypto,}) {final _that = this;
switch (_that) {
case CoreError_Network() when network != null:
return network(_that.message);case CoreError_GitHub() when gitHub != null:
return gitHub(_that.status,_that.message);case CoreError_Integrity() when integrity != null:
return integrity(_that.message);case CoreError_InvalidInput() when invalidInput != null:
return invalidInput(_that.message);case CoreError_Io() when io != null:
return io(_that.message);case CoreError_DecryptFailed() when decryptFailed != null:
return decryptFailed();case CoreError_Crypto() when crypto != null:
return crypto();case _:
  return null;

}
}

}

/// @nodoc


class CoreError_Network extends CoreError {
  const CoreError_Network({required this.message}): super._();
  

 final  String message;

/// Create a copy of CoreError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$CoreError_NetworkCopyWith<CoreError_Network> get copyWith => _$CoreError_NetworkCopyWithImpl<CoreError_Network>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is CoreError_Network&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode {
    return Object.hash(runtimeType,message);
}

@override
String toString() {
    return 'CoreError.network(message: $message)';
}


}

/// @nodoc
abstract mixin class $CoreError_NetworkCopyWith<$Res> implements $CoreErrorCopyWith<$Res> {
  factory $CoreError_NetworkCopyWith(CoreError_Network value, $Res Function(CoreError_Network) _then) = _$CoreError_NetworkCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$CoreError_NetworkCopyWithImpl<$Res>
    implements $CoreError_NetworkCopyWith<$Res> {
  _$CoreError_NetworkCopyWithImpl(this._self, this._then);

  final CoreError_Network _self;
  final $Res Function(CoreError_Network) _then;

/// Create a copy of CoreError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(CoreError_Network(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class CoreError_GitHub extends CoreError {
  const CoreError_GitHub({required this.status, required this.message}): super._();
  

 final  int status;
 final  String message;

/// Create a copy of CoreError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$CoreError_GitHubCopyWith<CoreError_GitHub> get copyWith => _$CoreError_GitHubCopyWithImpl<CoreError_GitHub>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is CoreError_GitHub&&(identical(other.status, status) || other.status == status)&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode {
    return Object.hash(runtimeType,status,message);
}

@override
String toString() {
    return 'CoreError.gitHub(status: $status, message: $message)';
}


}

/// @nodoc
abstract mixin class $CoreError_GitHubCopyWith<$Res> implements $CoreErrorCopyWith<$Res> {
  factory $CoreError_GitHubCopyWith(CoreError_GitHub value, $Res Function(CoreError_GitHub) _then) = _$CoreError_GitHubCopyWithImpl;
@useResult
$Res call({
 int status, String message
});




}
/// @nodoc
class _$CoreError_GitHubCopyWithImpl<$Res>
    implements $CoreError_GitHubCopyWith<$Res> {
  _$CoreError_GitHubCopyWithImpl(this._self, this._then);

  final CoreError_GitHub _self;
  final $Res Function(CoreError_GitHub) _then;

/// Create a copy of CoreError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? status = null,Object? message = null,}) {
  return _then(CoreError_GitHub(
status: null == status ? _self.status : status // ignore: cast_nullable_to_non_nullable
as int,message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class CoreError_Integrity extends CoreError {
  const CoreError_Integrity({required this.message}): super._();
  

 final  String message;

/// Create a copy of CoreError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$CoreError_IntegrityCopyWith<CoreError_Integrity> get copyWith => _$CoreError_IntegrityCopyWithImpl<CoreError_Integrity>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is CoreError_Integrity&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode {
    return Object.hash(runtimeType,message);
}

@override
String toString() {
    return 'CoreError.integrity(message: $message)';
}


}

/// @nodoc
abstract mixin class $CoreError_IntegrityCopyWith<$Res> implements $CoreErrorCopyWith<$Res> {
  factory $CoreError_IntegrityCopyWith(CoreError_Integrity value, $Res Function(CoreError_Integrity) _then) = _$CoreError_IntegrityCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$CoreError_IntegrityCopyWithImpl<$Res>
    implements $CoreError_IntegrityCopyWith<$Res> {
  _$CoreError_IntegrityCopyWithImpl(this._self, this._then);

  final CoreError_Integrity _self;
  final $Res Function(CoreError_Integrity) _then;

/// Create a copy of CoreError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(CoreError_Integrity(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class CoreError_InvalidInput extends CoreError {
  const CoreError_InvalidInput({required this.message}): super._();
  

 final  String message;

/// Create a copy of CoreError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$CoreError_InvalidInputCopyWith<CoreError_InvalidInput> get copyWith => _$CoreError_InvalidInputCopyWithImpl<CoreError_InvalidInput>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is CoreError_InvalidInput&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode {
    return Object.hash(runtimeType,message);
}

@override
String toString() {
    return 'CoreError.invalidInput(message: $message)';
}


}

/// @nodoc
abstract mixin class $CoreError_InvalidInputCopyWith<$Res> implements $CoreErrorCopyWith<$Res> {
  factory $CoreError_InvalidInputCopyWith(CoreError_InvalidInput value, $Res Function(CoreError_InvalidInput) _then) = _$CoreError_InvalidInputCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$CoreError_InvalidInputCopyWithImpl<$Res>
    implements $CoreError_InvalidInputCopyWith<$Res> {
  _$CoreError_InvalidInputCopyWithImpl(this._self, this._then);

  final CoreError_InvalidInput _self;
  final $Res Function(CoreError_InvalidInput) _then;

/// Create a copy of CoreError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(CoreError_InvalidInput(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class CoreError_Io extends CoreError {
  const CoreError_Io({required this.message}): super._();
  

 final  String message;

/// Create a copy of CoreError
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$CoreError_IoCopyWith<CoreError_Io> get copyWith => _$CoreError_IoCopyWithImpl<CoreError_Io>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is CoreError_Io&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode {
    return Object.hash(runtimeType,message);
}

@override
String toString() {
    return 'CoreError.io(message: $message)';
}


}

/// @nodoc
abstract mixin class $CoreError_IoCopyWith<$Res> implements $CoreErrorCopyWith<$Res> {
  factory $CoreError_IoCopyWith(CoreError_Io value, $Res Function(CoreError_Io) _then) = _$CoreError_IoCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$CoreError_IoCopyWithImpl<$Res>
    implements $CoreError_IoCopyWith<$Res> {
  _$CoreError_IoCopyWithImpl(this._self, this._then);

  final CoreError_Io _self;
  final $Res Function(CoreError_Io) _then;

/// Create a copy of CoreError
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(CoreError_Io(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class CoreError_DecryptFailed extends CoreError {
  const CoreError_DecryptFailed(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is CoreError_DecryptFailed);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'CoreError.decryptFailed()';
}


}




/// @nodoc


class CoreError_Crypto extends CoreError {
  const CoreError_Crypto(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is CoreError_Crypto);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'CoreError.crypto()';
}


}




// dart format on
