// Shader compilation and ABI reflection through Slang's C++ API.
#include <cstdint>
#include <fstream>
#include <exception>
#include <iostream>
#include <set>
#include <string>
#include <vector>

#include "slang-com-ptr.h"
#include "slang.h"

enum class Target
{
    Spirv,
    Cuda,
    Metal,
    Host,
};

struct CompileRequest
{
    const char* directory;
    const char* module;
    Target target;
    const char* code_path;
    const char* reflection_path;
    const char* abi_path;
    const char* const* entries;
    size_t entry_count;
};

static constexpr const char* CUDA_CAPABILITY = "cuda_sm_8_0";
static constexpr const char* SPIRV_PROFILE = "spirv_1_5";

static const char* scalar_name(slang::TypeReflection::ScalarType scalar)
{
    switch (scalar)
    {
    case slang::TypeReflection::Int32:
        return "int32";
    case slang::TypeReflection::UInt32:
        return "uint32";
    case slang::TypeReflection::UInt8:
        return "uint8";
    case slang::TypeReflection::Float32:
        return "float32";
    default:
        return nullptr;
    }
}

static slang::Attribute* find_attribute(
    slang::VariableReflection* variable,
    const char* name)
{
    for (unsigned int index = 0; index < variable->getUserAttributeCount(); ++index)
    {
        auto attribute = variable->getUserAttributeByIndex(index);
        if (attribute && std::string(attribute->getName()) == name)
            return attribute;
    }
    return nullptr;
}

static bool reflect_variable_rust_type(
    slang::VariableReflection* variable,
    std::ostream& output)
{
    auto attribute = find_attribute(variable, "RustType");
    if (!attribute)
        return true;
    auto type = variable->getType();
    size_t size = 0;
    auto value = attribute->getArgumentValueString(0, &size);
    if (!value || !type || !type->getName())
    {
        std::cerr << "RustType requires one string argument and a named parameter type\n";
        return false;
    }
    output << "rust-type\t" << type->getName() << '\t'
           << std::string(value, size) << '\n';
    return true;
}

static bool reflect_enum(
    slang::TypeReflection* type,
    std::ostream& output,
    std::set<std::string>& reflected_enums)
{
    if (!reflected_enums.insert(type->getName()).second)
        return true;
    auto scalar = scalar_name(type->getElementType()->getScalarType());
    if (!scalar || std::string(scalar) == "float32")
    {
        std::cerr << "unsupported enum representation: " << type->getName() << '\n';
        return false;
    }
    for (unsigned int case_index = 0; case_index < type->getFieldCount(); ++case_index)
    {
        auto enum_case = type->getFieldByIndex(case_index);
        Slang::ComPtr<ISlangBlob> value_blob;
        if (SLANG_FAILED(enum_case->getDefaultValueBlob(value_blob.writeRef())) || !value_blob)
        {
            std::cerr << "cannot reflect enum case value: " << enum_case->getName() << '\n';
            return false;
        }
        int64_t value = 0;
        if (scalar == std::string("int32") && value_blob->getBufferSize() == sizeof(int32_t))
            value = *static_cast<const int32_t*>(value_blob->getBufferPointer());
        else if (scalar == std::string("uint32") && value_blob->getBufferSize() == sizeof(uint32_t))
            value = *static_cast<const uint32_t*>(value_blob->getBufferPointer());
        else if (scalar == std::string("uint8") && value_blob->getBufferSize() == sizeof(uint8_t))
            value = *static_cast<const uint8_t*>(value_blob->getBufferPointer());
        else
        {
            std::cerr << "unexpected enum case representation: " << enum_case->getName() << '\n';
            return false;
        }
        output << "enum\t" << type->getName() << '\t' << scalar << '\t'
               << enum_case->getName() << '\t' << value << '\n';
    }
    return true;
}

static bool reflect_type(
    slang::TypeLayoutReflection* layout,
    std::ostream& output,
    std::set<std::string>& reflected_structs,
    std::set<std::string>& reflected_enums)
{
    auto type = layout ? layout->getType() : nullptr;
    if (!type)
        return true;
    if (type->getKind() == slang::TypeReflection::Kind::ConstantBuffer
        || type->getKind() == slang::TypeReflection::Kind::ParameterBlock)
        return reflect_type(
            layout->getElementTypeLayout(), output, reflected_structs, reflected_enums);
    if (type->getKind() == slang::TypeReflection::Kind::Struct)
    {
        if (!reflected_structs.insert(type->getName()).second)
            return true;
        if (auto attribute = type->findUserAttributeByName("RustType"))
        {
            size_t size = 0;
            auto value = attribute->getArgumentValueString(0, &size);
            if (!value)
            {
                std::cerr << "RustType requires one string argument: " << type->getName() << '\n';
                return false;
            }
            output << "rust-type\t" << type->getName() << '\t'
                   << std::string(value, size) << '\n';
        }
        for (unsigned int field_index = 0; field_index < layout->getFieldCount(); ++field_index)
        {
            auto field_layout = layout->getFieldByIndex(field_index);
            auto field = field_layout->getVariable();
            auto field_type = field->getType();
            if (field_type->getKind() == slang::TypeReflection::Kind::Enum)
            {
                output << "enum-field\t" << type->getName() << '\t'
                       << field->getName() << '\t' << field_type->getName() << '\n';
                if (!reflect_enum(field_type, output, reflected_enums))
                    return false;
            }
            if (field_type->getKind() == slang::TypeReflection::Kind::Pointer)
            {
                auto value_layout = field_layout->getTypeLayout()->getElementTypeLayout();
                auto value_type = value_layout ? value_layout->getType() : nullptr;
                output << "pointer-field\t" << type->getName() << '\t'
                       << field->getName() << '\t';
                if (!value_type)
                {
                    std::cerr << "cannot reflect pointer target: " << type->getName()
                              << '.' << field->getName() << '\n';
                    return false;
                }
                if (value_type->getKind() == slang::TypeReflection::Kind::Scalar)
                {
                    auto scalar = scalar_name(value_type->getScalarType());
                    if (!scalar)
                        return false;
                    output << "scalar\t" << scalar << '\n';
                }
                else if (value_type->getKind() == slang::TypeReflection::Kind::Vector)
                {
                    auto scalar = scalar_name(value_type->getElementType()->getScalarType());
                    if (!scalar)
                        return false;
                    output << "vector\t" << scalar << '\t'
                           << value_type->getElementCount() << '\n';
                }
                else if (value_type->getKind() == slang::TypeReflection::Kind::Struct)
                    output << "struct\t" << value_type->getName() << '\n';
                else
                {
                    std::cerr << "unsupported pointer target: " << type->getName()
                              << '.' << field->getName() << '\n';
                    return false;
                }
            }
            if (!reflect_type(
                    field_layout->getTypeLayout(),
                    output,
                    reflected_structs,
                    reflected_enums))
                return false;
        }
        return true;
    }
    if (type->getKind() != slang::TypeReflection::Kind::Enum)
        return true;
    return reflect_enum(type, output, reflected_enums);
}

static int compile(const CompileRequest& request)
{
    const bool cuda = request.target == Target::Cuda;
    const bool spirv = request.target == Target::Spirv;

    Slang::ComPtr<slang::IGlobalSession> global_session;
    if (SLANG_FAILED(slang::createGlobalSession(global_session.writeRef())))
        return 1;

    const std::string module_path = std::string(request.directory) + "/modules";
    const char* search_paths[] = {request.directory, module_path.c_str()};
    std::vector<slang::CompilerOptionEntry> options;
    const auto option = [&](slang::CompilerOptionName name, int value) {
        slang::CompilerOptionEntry entry = {};
        entry.name = name;
        entry.value.intValue0 = value;
        options.push_back(entry);
    };
    option(slang::CompilerOptionName::Optimization, SLANG_OPTIMIZATION_LEVEL_HIGH);
    if (cuda)
        option(slang::CompilerOptionName::Capability, global_session->findCapability(CUDA_CAPABILITY));
    if (spirv)
    {
        option(slang::CompilerOptionName::Capability, global_session->findCapability("spvGroupNonUniform"));
        option(slang::CompilerOptionName::Capability, global_session->findCapability("spvGroupNonUniformBallot"));
        option(slang::CompilerOptionName::EmitSpirvMethod, SLANG_EMIT_SPIRV_DIRECTLY);
    }
    slang::TargetDesc target = {};
    switch (request.target)
    {
    case Target::Spirv: target.format = SLANG_SPIRV; break;
    case Target::Cuda: target.format = SLANG_CUDA_SOURCE; break;
    case Target::Metal: target.format = SLANG_METAL; break;
    case Target::Host: target.format = SLANG_CPP_SOURCE; break;
    }
    target.flags = SLANG_TARGET_FLAG_GENERATE_WHOLE_PROGRAM;
    target.compilerOptionEntries = options.data();
    target.compilerOptionEntryCount = static_cast<uint32_t>(options.size());
    if (cuda)
        target.floatingPointMode = SLANG_FLOATING_POINT_MODE_PRECISE;
    if (spirv)
        target.profile = global_session->findProfile(SPIRV_PROFILE);
    slang::SessionDesc description = {};
    description.defaultMatrixLayoutMode = cuda || request.target == Target::Host
        ? SLANG_MATRIX_LAYOUT_ROW_MAJOR : SLANG_MATRIX_LAYOUT_COLUMN_MAJOR;
    description.searchPathCount = std::size(search_paths);
    description.searchPaths = search_paths;
    description.targetCount = 1;
    description.targets = &target;
    Slang::ComPtr<slang::ISession> session;
    if (SLANG_FAILED(global_session->createSession(description, session.writeRef())))
        return 1;

    Slang::ComPtr<slang::IBlob> diagnostics;
    Slang::ComPtr<slang::IModule> module;
    module = session->loadModule(request.module, diagnostics.writeRef());
    if (diagnostics)
        std::cerr << static_cast<const char*>(diagnostics->getBufferPointer());
    if (!module)
        return 1;

    std::vector<Slang::ComPtr<slang::IEntryPoint>> entries;
    const bool explicit_entries = request.entry_count != 0;
    const auto entry_count = explicit_entries ? request.entry_count : static_cast<size_t>(module->getDefinedEntryPointCount());
    for (size_t index = 0; index < entry_count; ++index)
    {
        Slang::ComPtr<slang::IEntryPoint> entry;
        const auto result = explicit_entries
            ? module->findEntryPointByName(request.entries[index], entry.writeRef())
            : module->getDefinedEntryPoint(static_cast<SlangInt32>(index), entry.writeRef());
        if (SLANG_FAILED(result))
        {
            std::cerr << "cannot find shader entry point\n";
            return 1;
        }
        entries.push_back(entry);
    }
    if (entries.empty())
    {
        std::cerr << "shader module has no entry points\n";
        return 1;
    }
    std::vector<slang::IComponentType*> components = { module };
    for (const auto& entry : entries)
        components.push_back(entry);
    const auto diagnosed = [&](SlangResult result) {
        if (diagnostics)
            std::cerr << static_cast<const char*>(diagnostics->getBufferPointer());
        diagnostics.setNull();
        return SLANG_SUCCEEDED(result);
    };
    Slang::ComPtr<slang::IComponentType> composite;
    if (!diagnosed(session->createCompositeComponentType(
            components.data(), components.size(), composite.writeRef(), diagnostics.writeRef())))
        return 1;
    Slang::ComPtr<slang::IComponentType> program;
    if (!diagnosed(composite->link(program.writeRef(), diagnostics.writeRef())))
        return 1;

    std::ofstream output(request.abi_path);
    if (!output)
        return 1;
    std::set<std::string> reflected_structs;
    std::set<std::string> reflected_enums;
    auto layout = program->getLayout(0, diagnostics.writeRef());
    if (diagnostics)
        std::cerr << static_cast<const char*>(diagnostics->getBufferPointer());
    if (!layout)
        return 1;
    Slang::ComPtr<slang::IBlob> code;
    if (!diagnosed(program->getTargetCode(0, code.writeRef(), diagnostics.writeRef())))
        return 1;
    Slang::ComPtr<slang::IBlob> json;
    if (SLANG_FAILED(layout->toJson(json.writeRef())))
        return 1;
    std::ofstream code_file(request.code_path, std::ios::binary);
    code_file.write(static_cast<const char*>(code->getBufferPointer()), code->getBufferSize());
    std::ofstream json_file(request.reflection_path, std::ios::binary);
    json_file.write(static_cast<const char*>(json->getBufferPointer()), json->getBufferSize());
    if (!code_file || !json_file)
        return 1;
    for (unsigned int parameter_index = 0;
         parameter_index < layout->getParameterCount();
         ++parameter_index)
    {
        auto parameter = layout->getParameterByIndex(parameter_index);
        if (!reflect_type(
                parameter->getTypeLayout(), output, reflected_structs, reflected_enums))
            return 1;
    }
    for (unsigned int entry_index = 0; entry_index < layout->getEntryPointCount(); ++entry_index)
    {
        auto entry = layout->getEntryPointByIndex(entry_index);
        for (unsigned int parameter_index = 0;
             parameter_index < entry->getParameterCount();
             ++parameter_index)
        {
            auto parameter_layout = entry->getParameterByIndex(parameter_index);
            auto parameter = parameter_layout->getVariable();
            if (!reflect_variable_rust_type(parameter, output))
                return 1;
            if (!reflect_type(
                    parameter_layout->getTypeLayout(),
                    output,
                    reflected_structs,
                    reflected_enums))
                return 1;
        }
    }
    return output ? 0 : 1;
}

extern "C" int shrimply_slang_compile(const CompileRequest* request) noexcept
{
    try
    {
        return compile(*request);
    }
    catch (const std::exception& error)
    {
        std::cerr << "Slang API exception: " << error.what() << '\n';
        return 1;
    }
    catch (...)
    {
        std::cerr << "Slang API exception\n";
        return 1;
    }
}
